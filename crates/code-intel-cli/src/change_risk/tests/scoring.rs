use std::collections::BTreeSet;

use super::super::git::{commits_in_range, diff_stats, resolve_repo_root_from};
use super::super::*;
use super::*;

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
