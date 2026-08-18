use super::super::git::tip_token;
use super::super::signals::{is_source_file, is_test_file, looks_like_fix_subject};
use super::super::*;

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
fn is_test_file_credits_this_crates_own_flat_module_tests_suffix_convention() {
    // `artifacts.rs` -> `artifacts_tests.rs`, `model.rs` -> `model_tests.rs`,
    // and so on for every flat (non-directory) multi-part module in this
    // crate: nine existing files use this plural suffix, zero use the
    // singular `_test.rs` the heuristic checked before this.
    for path in [
        "crates/code-intel-cli/src/artifacts_tests.rs",
        "crates/code-intel-cli/src/boundary_rules_tests.rs",
        "crates/code-intel-cli/src/audit_report/model_tests.rs",
    ] {
        assert!(is_test_file(path), "{path}");
    }
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

/// Same contract as the bare-`tests.rs` case above, but for the plural
/// `_tests.rs` suffix `is_test_file` was just taught to credit -- proving
/// `score_subset` actually consumes the wider predicate, not just that the
/// predicate itself returns `true` in isolation.
#[test]
fn a_change_touching_a_plural_tests_module_is_not_scored_as_asymmetric() {
    let files = vec![
        (
            40,
            2,
            "crates/code-intel-cli/src/change_risk/signals.rs".to_string(),
        ),
        (
            30,
            0,
            "crates/code-intel-cli/src/artifacts_tests.rs".to_string(),
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
        .find(|file| file["path"] == "crates/code-intel-cli/src/artifacts_tests.rs")
        .expect("the artifacts_tests.rs row must be reported");
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
/// The workflow that consumes this module had no test at all, which is how
/// issue #201 lived in it unnoticed: the gate judged `risk_percentile`, a rank
/// against a rolling sample, so a pull request could turn red because its
/// neighbours got smaller rather than because it got riskier — and ~10% of
/// commits sit above the 90th percentile by construction, making the gate a
/// quota no amount of repository health could clear.
///
/// This pins the contract, not the bytes: which field decides, that the
/// percentile is reported but never gated, and that the threshold sits in the
/// measured gap between the dense 66-69 cluster and the 90+ outliers. A silent
/// revert to percentile gating, or a threshold quietly tuned down into the
/// cluster to unblock something in flight, fails here.
#[test]
fn the_pr_gate_blocks_on_the_absolute_score_not_a_moving_rank() {
    const WORKFLOW: &str = include_str!("../../../../../.github/workflows/pr-gate.yml");

    assert!(
        WORKFLOW.contains("RISK_SCORE_BLOCK:"),
        "pr-gate.yml must declare an absolute score threshold"
    );
    assert!(
        !WORKFLOW.contains("RISK_PERCENTILE_BLOCK"),
        "the percentile threshold is retired (#201); a reintroduced one gates on a moving denominator"
    );

    let threshold: f64 = WORKFLOW
        .lines()
        .find_map(|line| line.trim().strip_prefix("RISK_SCORE_BLOCK:"))
        .map(|value| value.trim().trim_matches('"').to_string())
        .expect("pr-gate.yml declares RISK_SCORE_BLOCK")
        .parse()
        .expect("RISK_SCORE_BLOCK is numeric");
    assert!(
        (70.0..=89.0).contains(&threshold),
        "the threshold must stay inside the measured empty band between the 66-69 cluster and \
         the 90+ outliers; re-derive the distribution before moving it outside: {threshold}"
    );

    // The gate reads `.score`; `risk_percentile` may only be narrated.
    let gate_step = WORKFLOW
        .split("- name: Evaluate gate")
        .nth(1)
        .expect("pr-gate.yml has an Evaluate gate step");
    assert!(
        gate_step.contains("'.score'") || gate_step.contains(".score >="),
        "the gate step must decide on the absolute score"
    );
    assert!(
        !gate_step.contains("risk_percentile"),
        "the gate step must not read risk_percentile (#201)"
    );

    // `score` is a JSON number and `round2` can emit a fraction, which shell
    // `-ge` rejects outright; under `bash -e` that aborts the job instead of
    // deciding, so the comparison has to happen in jq.
    assert!(
        gate_step.contains("jq --argjson threshold"),
        "the score comparison must run in jq, not shell -ge, so a fractional score cannot abort \
         the job instead of blocking or passing"
    );
}
