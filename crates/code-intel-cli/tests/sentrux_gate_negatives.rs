//! What the architecture gate must **not** flag.
//!
//! Every existing case for this gate is positive: build a violation, assert it
//! is caught. That direction alone cannot fail from over-flagging, so raising
//! sensitivity always looks like an improvement — the gate gets "stricter" and
//! every test still passes. The cost lands on whoever hits the false positive,
//! and it lands as a red build with no test explaining why the finding is
//! wrong.
//!
//! So each case here is a tree that *looks* like a violation and is not, paired
//! with the reason it is legitimate. A change that starts flagging one of these
//! has to argue with the rationale rather than silently ship.
//!
//! These are deliberately about the gate's *stance*, not its plumbing: they run
//! the shipped `sentrux --operation check` the way the self-scan and CI do.

mod common;

use std::fs;
use std::path::PathBuf;

fn fixture_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "sentrux-gate-neg-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src")).expect("create fixture root");
    root
}

fn code_intel(args: &[&str]) -> std::process::Output {
    common::cli().args(args).output().expect("run code-intel")
}

fn write_rules(root: &PathBuf) {
    fs::create_dir_all(root.join(".sentrux")).expect("create .sentrux");
    fs::write(
        root.join(".sentrux/rules.toml"),
        "[constraints]\nmax_cycles = 0\nno_god_files = false\n",
    )
    .expect("write rules.toml");
}

fn padded(head: &str, lines: usize) -> String {
    let mut body = String::from(head);
    for line in 0..lines {
        body.push_str(&format!("// padding {line}\n"));
    }
    body
}

fn save_baseline(root: &PathBuf) {
    let root_arg = root.to_string_lossy().to_string();
    let saved = code_intel(&[
        "sentrux",
        "--operation",
        "save_baseline",
        "--repo",
        &root_arg,
    ]);
    assert!(
        saved.status.success(),
        "baseline save failed: {}{}",
        String::from_utf8_lossy(&saved.stdout),
        String::from_utf8_lossy(&saved.stderr),
    );
}

/// Run `check` and assert it stays green, quoting the rationale on failure.
fn assert_not_flagged(root: &PathBuf, why: &str) {
    let root_arg = root.to_string_lossy().to_string();
    let check = code_intel(&["sentrux", "--operation", "check", "--repo", &root_arg]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr),
    );
    assert!(
        check.status.success(),
        "the gate flagged a tree it should not have.\n\
         Why this is legitimate: {why}\n\
         Gate said:\n{combined}",
    );
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn growth_that_stays_under_the_threshold_is_not_a_regression() {
    // A file getting longer is not itself the finding — crossing the rule is.
    // A gate that trends on size rather than thresholds turns every ordinary
    // commit into a negotiation.
    let root = fixture_root("under-threshold");
    fs::write(root.join("src/lib.rs"), padded("pub fn entry() {}\n", 200))
        .expect("write initial file");
    write_rules(&root);
    save_baseline(&root);

    fs::write(root.join("src/lib.rs"), padded("pub fn entry() {}\n", 700))
        .expect("grow the file");

    assert_not_flagged(
        &root,
        "the file grew by 500 lines but is still under loc>800; growth below the \
         threshold is ordinary work, and flagging it makes the threshold meaningless",
    );
}

#[test]
fn a_grandfathered_god_file_that_shrinks_is_not_a_regression() {
    // Debt paid down must never read as debt added. This is the case that
    // breaks if the ratchet ever compares counts instead of identities.
    let root = fixture_root("shrinking-god");
    fs::write(root.join("src/big.rs"), padded("pub fn entry() {}\n", 900))
        .expect("write god file");
    write_rules(&root);
    save_baseline(&root);

    fs::write(root.join("src/big.rs"), padded("pub fn entry() {}\n", 820))
        .expect("shrink the god file");

    assert_not_flagged(
        &root,
        "the file is still a god file but smaller than the baseline recorded; \
         paying debt down must not read as adding it",
    );
}

#[test]
fn deleting_a_god_file_and_adding_a_small_one_is_not_a_regression() {
    // The count moves 1 -> 1 here only if the replacement is also a god file.
    // It is not, so this is a strict improvement that a count-based rule with a
    // sloppy identity check could still report as churn.
    let root = fixture_root("god-replaced-small");
    fs::write(root.join("src/big.rs"), padded("pub fn entry() {}\n", 900))
        .expect("write god file");
    write_rules(&root);
    save_baseline(&root);

    fs::remove_file(root.join("src/big.rs")).expect("remove god file");
    fs::write(root.join("src/small.rs"), "pub fn small() {}\n").expect("write small file");

    assert_not_flagged(
        &root,
        "a god file was deleted and replaced by a small one — the god file count \
         went 1 -> 0, which is unambiguously an improvement",
    );
}

#[test]
fn adding_tests_to_an_existing_file_is_not_a_new_god_file() {
    // The one that bites in practice: a test module grows past the threshold
    // because someone added coverage. Flagging it prices new tests at the cost
    // of a refactor, so the cheapest way to keep the gate green becomes writing
    // fewer tests — the exact opposite of what the gate is for.
    //
    // The baseline already lists this path, so the identity ratchet tolerates
    // it as standing debt. This case exists to keep that true.
    let root = fixture_root("tests-grew");
    fs::create_dir_all(root.join("tests")).expect("create tests dir");
    fs::write(
        root.join("tests/suite.rs"),
        padded("#[test]\nfn one() {}\n", 850),
    )
    .expect("write test file");
    write_rules(&root);
    save_baseline(&root);

    fs::write(
        root.join("tests/suite.rs"),
        padded("#[test]\nfn one() {}\n#[test]\nfn two() {}\n", 1200),
    )
    .expect("add more tests");

    assert_not_flagged(
        &root,
        "a test file listed in the baseline grew because coverage was added; \
         charging new tests the price of a refactor makes 'write fewer tests' \
         the cheapest way to stay green",
    );
}

#[test]
fn the_negatives_above_are_not_vacuous() {
    // A negative corpus is worthless if the gate would have stayed green
    // anyway — five passing tests would then be measuring nothing. This is the
    // control: the same shape as `adding_tests_to_an_existing_file`, except the
    // baseline does not list the path, which is the one difference that should
    // decide the verdict. It must be flagged.
    let root = fixture_root("control-must-flag");
    fs::create_dir_all(root.join("tests")).expect("create tests dir");
    fs::write(root.join("src/small.rs"), "pub fn small() {}\n").expect("write small file");
    write_rules(&root);
    save_baseline(&root);

    fs::write(
        root.join("tests/suite.rs"),
        padded("#[test]\nfn one() {}\n", 1200),
    )
    .expect("write an unlisted god file");

    let root_arg = root.to_string_lossy().to_string();
    let check = code_intel(&["sentrux", "--operation", "check", "--repo", &root_arg]);
    assert!(
        !check.status.success(),
        "a god file absent from the baseline must be flagged — if this passes, \
         every negative case in this file is vacuous:\n{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr),
    );

    fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn an_unchanged_tree_is_never_a_regression() {
    // The floor. If this ever fails the gate is non-deterministic, and every
    // other negative here is unreadable noise.
    let root = fixture_root("unchanged");
    fs::write(root.join("src/big.rs"), padded("pub fn entry() {}\n", 900))
        .expect("write god file");
    fs::write(root.join("src/small.rs"), "pub fn small() {}\n").expect("write small file");
    write_rules(&root);
    save_baseline(&root);

    assert_not_flagged(
        &root,
        "nothing changed between baseline and check; a gate that fails here is \
         reporting its own nondeterminism, not a property of the code",
    );
}
