use super::super::inline_tests::{hunks_hit, inline_test_ranges, parse_hunks};
use super::super::*;
use super::*;

const FILE: &str = "crates/code-intel-cli/src/native_code_evidence.rs";

const BEFORE: &str = "pub fn compute() -> i32 {
    1
}

#[cfg(test)]
mod tests {
    #[test]
    fn works() {
        assert_eq!(1, 1);
    }
}
";

const AFTER_BOTH: &str = "pub fn compute() -> i32 {
    2
}

#[cfg(test)]
mod tests {
    #[test]
    fn works() {
        assert_eq!(1, 1);
    }

    #[test]
    fn also_works() {
        assert_eq!(2, 2);
    }
}
";

const AFTER_OUTSIDE: &str = "pub fn compute() -> i32 {
    2
}

#[cfg(test)]
mod tests {
    #[test]
    fn works() {
        assert_eq!(1, 1);
    }
}
";

fn two_commit_repo(name: &str, second: &str) -> PathBuf {
    let repo = init_repo(name);
    write_file(&repo, FILE, BEFORE);
    commit(&repo, "add compute");
    write_file(&repo, FILE, second);
    commit(&repo, "change compute");
    repo
}

fn signals_of(result: &Value) -> &Value {
    &result["signals"]["testAsymmetry"]
}

fn file_row<'a>(result: &'a Value, path: &str) -> &'a Value {
    result["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["path"] == path)
        .expect("file row present")
}

#[test]
fn fn_change_plus_inline_test_is_not_asymmetric() {
    let repo = two_commit_repo("inline-both", AFTER_BOTH);
    for spec in ["HEAD", "HEAD~1..HEAD"] {
        let result = execute(&repo, spec, 0).expect("scoring succeeds");
        let asymmetry = signals_of(&result);
        assert_eq!(asymmetry["sourceFilesChanged"], 1, "{spec}");
        assert_eq!(asymmetry["testFilesChanged"], 1, "{spec}");
        assert_eq!(asymmetry["asymmetric"], false, "{spec}");
        assert_eq!(asymmetry["subscore"], 0.0, "{spec}");
        let row = file_row(&result, FILE);
        assert_eq!(row["isTestFile"], false);
        assert_eq!(row["touchesInlineTests"], true);
    }
}

#[test]
fn change_outside_the_test_module_stays_asymmetric() {
    let repo = two_commit_repo("inline-outside", AFTER_OUTSIDE);
    let result = execute(&repo, "HEAD", 0).expect("scoring succeeds");
    let asymmetry = signals_of(&result);
    assert_eq!(asymmetry["testFilesChanged"], 0);
    assert_eq!(asymmetry["asymmetric"], true);
    assert_eq!(file_row(&result, FILE)["touchesInlineTests"], false);
}

#[test]
fn every_file_row_carries_touches_inline_tests() {
    let repo = init_repo("inline-row-key");
    write_file(&repo, "README.md", "x\n");
    commit(&repo, "root");
    let result = execute(&repo, "HEAD", 0).expect("scoring succeeds");
    assert_eq!(file_row(&result, "README.md")["touchesInlineTests"], false);
}

#[test]
fn production_code_after_the_module_and_braces_in_strings() {
    let before = "#[cfg(test)]
mod tests {
    #[test]
    fn brace() {
        let s = \"{\";
        assert_eq!(s.len(), 1);
    }
}

pub fn after() -> i32 {
    1
}
";
    let repo = init_repo("inline-after");
    write_file(&repo, FILE, before);
    commit(&repo, "add");
    write_file(&repo, FILE, &before.replace("    1\n}", "    2\n}"));
    commit(&repo, "change after");
    let result = execute(&repo, "HEAD", 0).expect("scoring succeeds");
    assert_eq!(signals_of(&result)["asymmetric"], true);
    assert_eq!(file_row(&result, FILE)["touchesInlineTests"], false);
}

#[test]
fn last_test_after_a_brace_comment_and_extra_attribute_counts() {
    let before = "pub fn f() {}

#[cfg(test)]
#[allow(clippy::all)]
mod tests {
    // a stray } in a comment
    #[test]
    fn last() {
        assert!(true);
    }
}
";
    let repo = init_repo("inline-attrs");
    write_file(&repo, FILE, before);
    commit(&repo, "add");
    write_file(
        &repo,
        FILE,
        &before.replace("assert!(true)", "assert!(!false)"),
    );
    commit(&repo, "edit last test");
    let result = execute(&repo, "HEAD", 0).expect("scoring succeeds");
    assert_eq!(signals_of(&result)["asymmetric"], false);
}

#[test]
fn deleting_a_test_from_the_module_counts_but_a_cfg_test_helper_fn_does_not() {
    let repo = init_repo("inline-delete");
    write_file(&repo, FILE, AFTER_BOTH);
    commit(&repo, "add");
    write_file(&repo, FILE, AFTER_OUTSIDE);
    commit(&repo, "drop a test");
    let result = execute(&repo, "HEAD", 0).expect("scoring succeeds");
    assert_eq!(signals_of(&result)["asymmetric"], false);

    let helper = "#[cfg(test)]
fn helper() -> i32 {
    1
}

pub fn f() {}
";
    let repo = init_repo("inline-helper");
    write_file(&repo, FILE, helper);
    commit(&repo, "add");
    write_file(&repo, FILE, &helper.replace("    1\n", "    2\n"));
    commit(&repo, "edit helper");
    let result = execute(&repo, "HEAD", 0).expect("scoring succeeds");
    assert_eq!(signals_of(&result)["asymmetric"], true);
}

#[test]
fn text_format_marks_inline_test_rows() {
    let repo = two_commit_repo("inline-text", AFTER_BOTH);
    let result = execute(&repo, "HEAD", 0).expect("scoring succeeds");
    let text = render::render_text(&result);
    let line = text
        .lines()
        .find(|line| line.contains(FILE))
        .expect("file line");
    assert!(line.ends_with(" [inline-test]"), "{line}");
}

#[test]
fn sample_baselines_use_the_same_inline_test_decision() {
    let other = "crates/code-intel-cli/src/other.rs";
    let repo = init_repo("inline-sample");
    write_file(&repo, FILE, BEFORE);
    write_file(&repo, other, "pub fn g() -> i32 {\n    1\n}\n");
    commit(&repo, "root");
    // Sample: source + inline test change only. With the fix it is not
    // asymmetric, so it scores lower than the source-only target.
    write_file(&repo, FILE, AFTER_BOTH);
    commit(&repo, "sample touches inline tests");
    write_file(&repo, other, "pub fn g() -> i32 {\n    2\n}\n");
    commit(&repo, "target touches other source only");
    let result = execute(&repo, "HEAD", 10).expect("scoring succeeds");
    assert_eq!(signals_of(&result)["asymmetric"], true);
    assert_eq!(result["risk_percentile"], 100);
}

#[test]
fn hunk_and_module_intersection() {
    let ranges = inline_test_ranges(BEFORE);
    assert_eq!(ranges, vec![(6, 11)]);
    let hit = |hunk| hunks_hit(&ranges, &ranges, &[hunk]);
    assert!(hit(((8, 1), (8, 1))));
    assert!(hit(((1, 1), (1, 6))), "overlap at the mod line");
    assert!(hit(((11, 1), (11, 3))), "overlap at the closing brace");
    assert!(!hit(((1, 1), (1, 3))));
    assert!(!hit(((12, 1), (12, 2))));
    assert!(hit(((8, 1), (7, 0))), "pure deletion of old lines inside");
    assert!(!hit(((3, 1), (2, 0))), "pure deletion of old lines outside");
    assert!(!hit(((12, 1), (11, 0))), "deletion just after the module");
    let new_only = hunks_hit(&ranges, &[], &[((7, 1), (6, 0))]);
    assert!(!new_only, "deletions never consult the new side");
}

#[test]
fn files_without_a_test_module_have_no_ranges() {
    assert!(inline_test_ranges("pub fn f() {}\n").is_empty());
    assert!(inline_test_ranges("#[cfg(test)]\nmod tests;\n").is_empty());
    assert!(inline_test_ranges("#[cfg(test)]\nfn helper() {}\n").is_empty());
    assert!(inline_test_ranges("#[cfg(all(test, unix))]\nmod tests {\n}\n").is_empty());
}

#[test]
fn braces_in_strings_comments_chars_and_lifetimes_do_not_count() {
    let source = "#[cfg(test)]
mod tests {
    fn a<'a>(x: &'a str) -> &'a str { x }
    const OPEN: char = '{';
    const CLOSE: char = '\\'';
    const RAW: &str = r#\"}}\"#;
    /* } /* } */ } */
    // }
    const S: &str = \"}\";
}
pub fn after() {}
";
    assert_eq!(inline_test_ranges(source), vec![(2, 10)]);
}

#[test]
fn parse_hunks_reads_both_sides() {
    let diff = "diff --git a/x.rs b/x.rs
index 1..2 100644
--- x.rs
+++ x.rs
@@ -3 +3 @@ fn
-a
+b
@@ -10,2 +10,0 @@
-c
-d
+++ not a header
diff --git a/y.rs b/y.rs
--- y.rs
+++ y.rs
@@ -0,0 +1,4 @@
";
    let hunks = parse_hunks(diff);
    assert_eq!(hunks["x.rs"], vec![((3, 1), (3, 1)), ((10, 2), (10, 0))]);
    assert_eq!(hunks["y.rs"], vec![((0, 0), (1, 4))]);
}

#[test]
fn deleting_production_code_right_after_the_module_stays_asymmetric() {
    let with_tail = format!("{BEFORE}\npub fn tail() -> i32 {{\n    3\n}}\n");
    let repo = init_repo("inline-delete-after");
    write_file(&repo, FILE, &with_tail);
    commit(&repo, "add");
    write_file(&repo, FILE, BEFORE);
    commit(&repo, "drop tail fn");
    for spec in ["HEAD", "HEAD~1..HEAD"] {
        let result = execute(&repo, spec, 0).expect("scoring succeeds");
        assert_eq!(signals_of(&result)["asymmetric"], true, "{spec}");
    }
}

#[test]
fn deleting_the_whole_test_module_counts_as_a_test_change() {
    let repo = init_repo("inline-delete-module");
    write_file(&repo, FILE, BEFORE);
    commit(&repo, "add");
    write_file(&repo, FILE, "pub fn compute() -> i32 {\n    2\n}\n");
    commit(&repo, "change fn and drop the test module");
    for spec in ["HEAD", "HEAD~1..HEAD"] {
        let result = execute(&repo, spec, 0).expect("scoring succeeds");
        assert_eq!(signals_of(&result)["testFilesChanged"], 1, "{spec}");
        assert_eq!(signals_of(&result)["asymmetric"], false, "{spec}");
    }
}

#[test]
fn spaced_cfg_test_attributes_give_ranges() {
    for attr in [
        "#[cfg( test )]",
        "#[ cfg ( test ) ]",
        "#[cfg(\n    test\n)]",
    ] {
        let source = format!("{attr}\nmod tests {{\n}}\n");
        assert!(!inline_test_ranges(&source).is_empty(), "{attr}");
    }
    assert!(inline_test_ranges("#[cfg( all(test, unix) )]\nmod tests {\n}\n").is_empty());
    assert!(inline_test_ranges("#[cfg(not(test))]\nmod tests {\n}\n").is_empty());
}

#[test]
fn visibility_qualified_test_modules_give_ranges() {
    for vis in [
        "pub",
        "pub(crate)",
        "pub (super)",
        "pub(self)",
        "pub(in crate::x)",
    ] {
        let source = format!("#[cfg(test)]\n#[allow(dead_code)]\n{vis} mod tests {{\n}}\n");
        assert_eq!(inline_test_ranges(&source), vec![(3, 4)], "{vis}");
    }
    assert!(inline_test_ranges("#[cfg(test)]\npubmod tests {\n}\n").is_empty());
}

#[test]
fn spaced_cfg_test_edit_inside_module_is_not_asymmetric() {
    let before = BEFORE.replace("#[cfg(test)]", "#[cfg( test )]");
    let repo = init_repo("inline-spaced");
    write_file(&repo, FILE, &before);
    commit(&repo, "add");
    write_file(
        &repo,
        FILE,
        &before.replace("assert_eq!(1, 1)", "assert_eq!(2, 2)"),
    );
    commit(&repo, "edit test");
    let result = execute(&repo, "HEAD", 0).expect("scoring succeeds");
    assert_eq!(signals_of(&result)["asymmetric"], false);
    assert_eq!(file_row(&result, FILE)["touchesInlineTests"], true);
}

#[test]
fn pub_crate_test_module_edit_is_not_asymmetric() {
    let before = BEFORE.replace("mod tests", "pub(crate) mod tests");
    let repo = init_repo("inline-pub-crate");
    write_file(&repo, FILE, &before);
    commit(&repo, "add");
    write_file(
        &repo,
        FILE,
        &before.replace("assert_eq!(1, 1)", "assert_eq!(2, 2)"),
    );
    commit(&repo, "edit test");
    let result = execute(&repo, "HEAD", 0).expect("scoring succeeds");
    assert_eq!(signals_of(&result)["asymmetric"], false);
}

#[test]
fn deleting_a_whole_file_with_a_test_module_counts_as_a_test_change() {
    let repo = init_repo("inline-delete-file");
    write_file(&repo, FILE, BEFORE);
    commit(&repo, "add");
    std::fs::remove_file(repo.join(FILE)).unwrap();
    commit(&repo, "delete file");
    for spec in ["HEAD", "HEAD~1..HEAD"] {
        let result = execute(&repo, spec, 0).expect("scoring succeeds");
        assert_eq!(signals_of(&result)["testFilesChanged"], 1, "{spec}");
        assert_eq!(signals_of(&result)["asymmetric"], false, "{spec}");
        assert_eq!(
            file_row(&result, FILE)["touchesInlineTests"],
            true,
            "{spec}"
        );
    }
}

#[test]
fn deleting_a_whole_file_without_a_test_module_stays_asymmetric() {
    let repo = init_repo("inline-delete-plain");
    write_file(&repo, FILE, "pub fn compute() -> i32 {\n    1\n}\n");
    commit(&repo, "add");
    std::fs::remove_file(repo.join(FILE)).unwrap();
    commit(&repo, "delete file");
    for spec in ["HEAD", "HEAD~1..HEAD"] {
        let result = execute(&repo, spec, 0).expect("scoring succeeds");
        assert_eq!(signals_of(&result)["asymmetric"], true, "{spec}");
        assert_eq!(
            file_row(&result, FILE)["touchesInlineTests"],
            false,
            "{spec}"
        );
    }
}

#[test]
fn parse_hunks_keys_a_deleted_file_by_its_old_path() {
    let diff = "diff --git x.rs x.rs
deleted file mode 100644
index 1..0
--- x.rs\t
+++ /dev/null
@@ -1,3 +0,0 @@
diff --git y.rs y.rs
new file mode 100644
--- /dev/null
+++ y.rs
@@ -0,0 +1,2 @@
";
    let hunks = parse_hunks(diff);
    assert_eq!(hunks["x.rs"], vec![((1, 3), (0, 0))]);
    assert_eq!(hunks["y.rs"], vec![((0, 0), (1, 2))]);
}
