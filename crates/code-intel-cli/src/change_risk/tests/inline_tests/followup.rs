use super::*;

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

#[test]
fn unparseable_tip_version_stays_asymmetric() {
    let repo = init_repo("inline-non-utf8-tip");
    let body = "pub fn compute() -> i32 {\n    1\n}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn works() {\n        assert_eq!(1, 1);\n    }\n}\n";
    write_file(&repo, FILE, body);
    commit(&repo, "add");
    let mut edited = b"// \xff not utf-8\n".to_vec();
    edited.extend_from_slice(body.replace("(1, 1)", "(2, 2)").as_bytes());
    std::fs::write(repo.join(FILE), edited).unwrap();
    commit(&repo, "edit");
    let result = execute(&repo, "HEAD", 0).expect("scoring succeeds");
    assert_eq!(signals_of(&result)["asymmetric"], true);
    assert_eq!(file_row(&result, FILE)["touchesInlineTests"], false);
}

#[test]
fn unparseable_tip_stays_asymmetric_when_another_path_becomes_a_directory() {
    let repo = init_repo("inline-batch-tree-tip");
    let other = "crates/code-intel-cli/src/other.rs";
    write_file(&repo, FILE, BEFORE);
    write_file(&repo, other, "pub fn other() {}\n");
    commit(&repo, "add");
    let mut edited = b"// \xff not utf-8\n".to_vec();
    edited.extend_from_slice(BEFORE.replace("(1, 1)", "(2, 2)").as_bytes());
    std::fs::write(repo.join(FILE), edited).unwrap();
    std::fs::remove_file(repo.join(other)).unwrap();
    write_file(&repo, &format!("{other}/mod.rs"), "pub fn other() {}\n");
    commit(&repo, "edit");
    let result = execute(&repo, "HEAD", 0).expect("scoring succeeds");
    assert_eq!(signals_of(&result)["asymmetric"], true);
}
