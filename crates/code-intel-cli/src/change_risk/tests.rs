use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use super::git::{commits_in_range, diff_stats, resolve_repo_root_from, tip_token};
use super::signals::{is_source_file, is_test_file, looks_like_fix_subject};
use super::*;

/// A fresh, empty temp directory namespaced the same way `init_repo` names
/// its repos, but without running `git init` — the starting point for both
/// a to-be-initialized repo and (for the invalid-`--repo` test) a directory
/// that must never look like one.
fn temp_dir_for(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "code-intel-change-risk-{name}-{nonce}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn init_repo(name: &str) -> PathBuf {
    let repo = temp_dir_for(name);
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

fn checkout_new_branch(repo: &Path, name: &str, start_point: Option<&str>) {
    let mut args = vec!["checkout", "--quiet", "-b", name];
    if let Some(start_point) = start_point {
        args.push(start_point);
    }
    assert!(Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .unwrap()
        .success());
}

fn rev_parse(repo: &Path, rev: &str) -> String {
    let output = Command::new("git")
        .args(["rev-parse", rev])
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).trim().to_string()
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
fn is_test_file_credits_this_crates_own_inline_test_module_convention() {
    // Every multi-part module here keeps its tests in a sibling `tests.rs`.
    // Before this was recognized, a PR adding one scored as "no tests
    // touched" — the heaviest single signal in the formula.
    for path in [
        "crates/code-intel-cli/src/change_risk/tests.rs",
        "crates/code-intel-cli/src/change_agenda/tests.rs",
        "crates/code-intel-cli/src/file_gate/tests.rs",
    ] {
        assert!(is_test_file(path), "{path}");
    }
}

#[test]
fn is_test_file_does_not_credit_names_that_merely_contain_tests() {
    // Whole-stem equality, not substring: these are ordinary source files.
    for path in [
        "crates/code-intel-cli/src/latest.rs",
        "crates/code-intel-cli/src/contests.rs",
        "crates/code-intel-cli/src/testsuite_helpers.rs",
    ] {
        assert!(!is_test_file(path), "{path}");
    }
}

/// Contract-level guard on the emitted signal, not just the predicate that
/// feeds it. The defect this fixes lived in what `testAsymmetry` reported,
/// so a green `is_test_file` alone would not have caught it — and would not
/// catch a future rewiring that leaves the predicate correct but stops
/// consulting it.
#[test]
fn a_change_touching_an_inline_tests_module_is_not_scored_as_asymmetric() {
    let files = vec![
        (
            40,
            2,
            "crates/code-intel-cli/src/change_risk/signals.rs".to_string(),
        ),
        (
            30,
            0,
            "crates/code-intel-cli/src/change_risk/tests.rs".to_string(),
        ),
    ];
    let scored = score_subset(&files, &FileHistory::new(), 1_700_000_000);

    let asymmetry = &scored.signals["testAsymmetry"];
    assert_eq!(asymmetry["testFilesChanged"].as_u64(), Some(1));
    assert_eq!(asymmetry["asymmetric"], false);
    assert_eq!(asymmetry["subscore"], 0.0);

    let tests_row = scored
        .files
        .iter()
        .find(|file| file["path"] == "crates/code-intel-cli/src/change_risk/tests.rs")
        .expect("the tests.rs row must be reported");
    assert_eq!(tests_row["isTestFile"], true);
}

/// The complementary half: the signal must still fire when a change really
/// does touch source without touching any test. A fix that made every change
/// look symmetric would pass the test above and silently disarm the largest
/// weight in the formula.
#[test]
fn a_source_only_change_is_still_scored_as_asymmetric() {
    let files = vec![(
        40,
        2,
        "crates/code-intel-cli/src/change_risk/signals.rs".to_string(),
    )];
    let scored = score_subset(&files, &FileHistory::new(), 1_700_000_000);

    let asymmetry = &scored.signals["testAsymmetry"];
    assert_eq!(asymmetry["testFilesChanged"].as_u64(), Some(0));
    assert_eq!(asymmetry["asymmetric"], true);
    assert_eq!(asymmetry["subscore"], 1.0);
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

    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn unresolvable_revspec_fails_closed_instead_of_looking_like_an_empty_diff() {
    let repo = init_repo("bogus-revspec");
    write_file(&repo, "README.md", "hello\n");
    commit(&repo, "chore: seed repository");

    let error = execute(&repo, "definitely-not-a-real-branch..HEAD", 5)
        .expect_err("an unresolvable revspec must fail the advisory computation");
    match error {
        RiskError::Contract(message) => {
            assert!(
                message.contains("cannot resolve change-risk revspec"),
                "{message}"
            )
        }
        RiskError::HostIo(message) => panic!("expected contract error, got host I/O: {message}"),
    }

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

#[test]
fn baseline_sampling_excludes_commits_inside_the_scored_range() {
    let repo = init_repo("baseline-self-contamination");
    write_file(&repo, "crates/demo/src/lib.rs", "fn a() {}\n");
    commit(&repo, "feat: c1"); // outside the scored range
    write_file(&repo, "crates/demo/src/lib.rs", "fn a() {}\nfn b() {}\n");
    commit(&repo, "feat: c2"); // outside the scored range
    write_file(
        &repo,
        "crates/demo/src/lib.rs",
        "fn a() {}\nfn b() {}\nfn c() {}\n",
    );
    commit(&repo, "feat: c3"); // outside the scored range
    write_file(
        &repo,
        "crates/demo/src/lib.rs",
        "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\n",
    );
    commit(&repo, "feat: c4"); // inside the scored range
    write_file(
        &repo,
        "crates/demo/src/lib.rs",
        "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn e() {}\n",
    );
    commit(&repo, "feat: c5"); // inside the scored range (tip)

    // Score only the last two commits, but request a baseline far larger
    // than the repository's total history (5 commits) so that, absent the
    // fix, `sample_history` walking back from the tip would pull every
    // commit reachable from HEAD — including the two commits that make up
    // the target's own diff — straight into the pool the target is being
    // compared against.
    let result = execute(&repo, "HEAD~2..HEAD", 10).expect("scoring succeeds");
    assert!(result.get("warning").is_none());
    // 5 commits total reachable from HEAD; the 2 inside HEAD~2..HEAD must
    // be filtered out of the baseline before scoring, leaving exactly the
    // 3 commits outside the range (each with a real diff of its own, c1
    // falling back to the empty-tree diff as the root commit).
    assert_eq!(
        result["signals"]["sampleUsed"], 3,
        "a commit inside the scored range must never also serve as one of its own baseline samples: {result}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn commits_in_range_normalizes_triple_dot_to_the_diffed_commit_set() {
    let repo = init_repo("triple-dot");
    write_file(&repo, "README.md", "base\n");
    commit(&repo, "chore: seed repository");

    // Diverge into two branches from the same commit: `base-branch` gets
    // one commit only it has, `feature-branch` gets two commits only it
    // has. `git diff base-branch...feature-branch` compares their merge
    // base to feature-branch's tip, so only feature-branch's two commits
    // should ever count as "the commits the diff was built from".
    checkout_new_branch(&repo, "base-branch", None);
    write_file(&repo, "base-only.txt", "base only\n");
    commit(&repo, "feat: base-only commit");
    let base_only_commit = rev_parse(&repo, "HEAD");

    checkout_new_branch(&repo, "feature-branch", Some("HEAD~1"));
    write_file(&repo, "feature-1.txt", "feature one\n");
    commit(&repo, "feat: feature commit one");
    let feature_first_commit = rev_parse(&repo, "HEAD");
    write_file(&repo, "feature-2.txt", "feature two\n");
    commit(&repo, "feat: feature commit two");
    let feature_second_commit = rev_parse(&repo, "HEAD");

    let excluded: BTreeSet<String> = commits_in_range(&repo, "base-branch...feature-branch")
        .into_iter()
        .collect();
    let expected: BTreeSet<String> = [feature_first_commit, feature_second_commit]
        .into_iter()
        .collect();
    assert_eq!(
        excluded, expected,
        "a...b must resolve to git diff's merge-base(a,b)..b commit set, not rev-list's symmetric difference"
    );
    assert!(
        !excluded.contains(&base_only_commit),
        "a commit unique to the base side of a...b must not appear in the exclusion set: {excluded:?}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn non_ascii_paths_arrive_unquoted_from_git() {
    let repo = init_repo("quotepath");
    write_file(&repo, "src/lib.rs", "fn base() {}\n");
    commit(&repo, "base");
    write_file(&repo, "src/统计模块.rs", "fn stats() {}\n");
    commit(&repo, "add non-ascii source file");

    let files = diff_stats(&repo, "HEAD^..HEAD").expect("diff should resolve");
    let paths: Vec<&str> = files.iter().map(|(_, _, path)| path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["src/统计模块.rs"],
        "with core.quotePath unset git C-quotes non-ASCII paths (\"src/\\347...\"), \
         which never match repository-relative lookup keys"
    );

    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn repo_flag_scores_a_fixture_repo_from_an_unrelated_cwd() {
    let repo = init_repo("repo-flag");
    write_file(&repo, "crates/demo/src/lib.rs", "pub fn demo() {}\n");
    commit(&repo, "feat: seed demo crate");
    write_file(
        &repo,
        "crates/demo/src/lib.rs",
        "pub fn demo() { println!(\"hi\"); }\n",
    );
    commit(&repo, "fix: correct demo output");

    // Deliberately do not touch this test process's own current directory:
    // it stays wherever `cargo test` started it (this crate's own
    // checkout), a repository entirely unrelated to the fixture just
    // created above. `--repo` must steer scoring at the fixture instead
    // (issue #114) — through the same entry point (`run_raw`) `main.rs`
    // actually dispatches to, not just the internal helpers.
    let args: Vec<String> = vec![
        "risk".to_string(),
        "HEAD~1..HEAD".to_string(),
        "--repo".to_string(),
        repo.to_string_lossy().into_owned(),
    ];

    assert_eq!(
        run_raw(&args),
        0,
        "the real change-risk entry point should succeed with --repo pointed at the fixture"
    );

    // `run_raw` only returns an exit code; re-run the typed parse + execute
    // path (same as the CLI adapter, without presentation) to inspect content.
    let request = ChangeRiskRequest::parse(&args).expect("--repo should parse");
    let result =
        execute_request(request).expect("scoring the fixture through --repo should succeed");
    let value = result.value();

    let expected_repo = resolve_repo_root_from(&repo)
        .expect("the fixture resolves its own Git root")
        .display()
        .to_string();
    assert_eq!(value["repo"], expected_repo);
    assert!(value.get("warning").is_none());
    assert_eq!(value["files"].as_array().unwrap().len(), 1);

    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn repo_flag_rejects_a_path_that_is_not_a_git_repository() {
    let not_a_repo = temp_dir_for("repo-flag-not-a-repo");

    let args: Vec<String> = vec![
        "risk".to_string(),
        "HEAD~1..HEAD".to_string(),
        "--repo".to_string(),
        not_a_repo.to_string_lossy().into_owned(),
    ];

    assert_eq!(
        run_raw(&args),
        65,
        "an invalid --repo must fail with the same Contract exit code (65) as every other \
         change-risk argument error, e.g. an unresolvable --sample or --format"
    );

    std::fs::remove_dir_all(&not_a_repo).ok();
}
