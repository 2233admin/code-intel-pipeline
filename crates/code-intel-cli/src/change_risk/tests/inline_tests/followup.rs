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

#[test]
fn top_of_file_deletion_is_not_a_whole_file_deletion() {
    let repo = init_repo("inline-top-deletion-unreadable-tip");
    let other = "crates/code-intel-cli/src/other.rs";
    let module = "#[cfg(test)]\nmod tests {\n    #[test]\n    fn works() {\n        assert_eq!(1, 1);\n    }\n}\n";
    write_file(&repo, FILE, module);
    write_file(&repo, other, "pub fn other() {}\n");
    commit(&repo, "add");
    // Drop the first two lines (hunk `@@ -1,2 +0,0 @@`), not the whole file.
    let kept: String = module.lines().skip(2).map(|l| format!("{l}\n")).collect();
    write_file(&repo, FILE, &kept);
    std::fs::remove_file(repo.join(other)).unwrap();
    write_file(&repo, &format!("{other}/mod.rs"), "pub fn other() {}\n");
    commit(&repo, "edit");
    let result = execute(&repo, "HEAD", 0).expect("scoring succeeds");
    assert_eq!(signals_of(&result)["asymmetric"], true);
}

#[test]
fn whitespace_between_hash_and_bracket_in_middle_attribute_gives_ranges() {
    assert_eq!(
        inline_test_ranges("#[cfg(test)]\n# [allow(dead_code)]\nmod tests {\n}\n"),
        vec![(3, 4)]
    );
    assert_eq!(
        inline_test_ranges("#[cfg(test)]\n#\n[allow(dead_code)]\npub mod tests {\n}\n"),
        vec![(4, 5)]
    );
    assert_eq!(
        inline_test_ranges(
            "#[cfg(test)]\n#[allow(dead_code)]\n# [allow(unused)]\nmod tests {\n}\n"
        ),
        vec![(4, 5)]
    );
}

#[test]
fn edit_inside_module_with_spaced_middle_attribute_touches_inline_tests() {
    let repo = init_repo("inline-spaced-middle-attr");
    let before = BEFORE.replace("#[cfg(test)]\n", "#[cfg(test)]\n# [allow(dead_code)]\n");
    write_file(&repo, FILE, &before);
    commit(&repo, "add");
    write_file(&repo, FILE, &before.replace("(1, 1)", "(2, 2)"));
    commit(&repo, "edit test");
    let result = execute(&repo, "HEAD", 0).expect("scoring succeeds");
    assert_eq!(signals_of(&result)["asymmetric"], false);
    assert_eq!(file_row(&result, FILE)["touchesInlineTests"], true);
}

const NON_ASCII_FILE: &str = "crates/code-intel-cli/src/测试.rs";

#[test]
fn non_ascii_path_edit_inside_test_module_touches_inline_tests() {
    let repo = init_repo("inline-non-ascii-edit");
    write_file(&repo, NON_ASCII_FILE, BEFORE);
    commit(&repo, "add");
    write_file(&repo, NON_ASCII_FILE, &BEFORE.replace("(1, 1)", "(2, 2)"));
    commit(&repo, "edit test");
    let result = execute(&repo, "HEAD", 0).expect("scoring succeeds");
    assert_eq!(signals_of(&result)["asymmetric"], false);
    assert_eq!(
        file_row(&result, NON_ASCII_FILE)["touchesInlineTests"],
        true
    );
}

#[test]
fn non_ascii_path_whole_file_deletion_counts_as_a_test_change() {
    let repo = init_repo("inline-non-ascii-delete");
    write_file(&repo, NON_ASCII_FILE, BEFORE);
    commit(&repo, "add");
    std::fs::remove_file(repo.join(NON_ASCII_FILE)).unwrap();
    commit(&repo, "delete file");
    let result = execute(&repo, "HEAD", 0).expect("scoring succeeds");
    assert_eq!(signals_of(&result)["testFilesChanged"], 1);
    assert_eq!(
        file_row(&result, NON_ASCII_FILE)["touchesInlineTests"],
        true
    );
}

#[test]
fn whole_file_deletion_with_unparseable_old_version_stays_asymmetric() {
    let repo = init_repo("inline-delete-non-utf8-old");
    let mut old = b"// \xff not utf-8\n".to_vec();
    old.extend_from_slice(BEFORE.as_bytes());
    std::fs::create_dir_all(repo.join(FILE).parent().unwrap()).unwrap();
    std::fs::write(repo.join(FILE), old).unwrap();
    commit(&repo, "add");
    std::fs::remove_file(repo.join(FILE)).unwrap();
    commit(&repo, "delete file");
    let result = execute(&repo, "HEAD", 0).expect("scoring succeeds");
    assert_eq!(signals_of(&result)["asymmetric"], true);
    assert_eq!(file_row(&result, FILE)["touchesInlineTests"], false);
}
