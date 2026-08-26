use super::*;

fn write_config(path: &Path, language: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, format!(r#"{{"language": "{language}"}}"#)).unwrap();
}

/// A user config path that resolves to nothing, for tiers that must
/// prove they work even when the tier below them is entirely absent.
fn absent_user_config(root: &Path) -> PathBuf {
    root.join("absent-user-config.json")
}

#[test]
fn explicit_flag_wins_over_every_configured_tier() {
    let root = crate::test_support::unique_temp_dir("flag-wins");
    write_config(&project_config_path(&root), "en");
    let user_config = root.join("user").join("config.json");
    write_config(&user_config, "en");

    let resolved = resolve_from(Some("zh"), Some(&root), &user_config, Some("en"));

    assert_eq!(resolved.language, "zh");
    assert_eq!(resolved.source, Source::Flag);
    fs::remove_dir_all(root).ok();
}

#[test]
fn project_config_wins_when_no_explicit_flag() {
    let root = crate::test_support::unique_temp_dir("project-wins");
    write_config(&project_config_path(&root), "zh");
    let user_config = root.join("user").join("config.json");
    write_config(&user_config, "en");

    let resolved = resolve_from(None, Some(&root), &user_config, Some("en"));

    assert_eq!(resolved.language, "zh");
    assert_eq!(resolved.source, Source::Project);
    fs::remove_dir_all(root).ok();
}

#[test]
fn user_config_wins_when_no_flag_and_no_project_config() {
    let root = crate::test_support::unique_temp_dir("user-wins");
    // No project config written at all: repo exists, but
    // `.code-intel/config.json` is absent.
    let user_config = root.join("user").join("config.json");
    write_config(&user_config, "en");

    let resolved = resolve_from(None, Some(&root), &user_config, Some("zh"));

    assert_eq!(resolved.language, "en");
    assert_eq!(resolved.source, Source::User);
    fs::remove_dir_all(root).ok();
}

#[test]
fn user_config_is_still_consulted_with_no_repo_at_all() {
    let root = crate::test_support::unique_temp_dir("user-wins-no-repo");
    let user_config = root.join("user").join("config.json");
    write_config(&user_config, "zh");

    let resolved = resolve_from(None, None, &user_config, Some("en"));

    assert_eq!(resolved.language, "zh");
    assert_eq!(resolved.source, Source::User);
    fs::remove_dir_all(root).ok();
}

#[test]
fn system_locale_wins_when_nothing_configured() {
    let root = crate::test_support::unique_temp_dir("locale-wins");
    let user_config = absent_user_config(&root);

    let resolved = resolve_from(None, Some(&root), &user_config, Some("zh"));

    assert_eq!(resolved.language, "zh");
    assert_eq!(resolved.source, Source::Locale);
    fs::remove_dir_all(root).ok();
}

#[test]
fn falls_back_to_en_when_no_tier_resolves() {
    let root = crate::test_support::unique_temp_dir("default-wins");
    let user_config = absent_user_config(&root);

    let resolved = resolve_from(None, Some(&root), &user_config, None);

    assert_eq!(resolved.language, "en");
    assert_eq!(resolved.source, Source::Default);
    fs::remove_dir_all(root).ok();
}

#[test]
fn an_explicit_flag_is_trimmed_but_never_validated_against_known_languages() {
    // Matches the flag's existing, already-unvalidated behavior: this
    // module only adds persistence and defaulting, not new validation.
    let resolved = resolve_from(Some("  fr  "), None, Path::new("does-not-exist"), None);
    assert_eq!(resolved.language, "fr");
    assert_eq!(resolved.source, Source::Flag);
}

#[test]
fn blank_explicit_flag_falls_through_instead_of_winning() {
    let root = crate::test_support::unique_temp_dir("blank-flag-falls-through");
    write_config(&project_config_path(&root), "zh");

    let resolved = resolve_from(Some("   "), Some(&root), &absent_user_config(&root), None);

    assert_eq!(resolved.language, "zh");
    assert_eq!(resolved.source, Source::Project);
    fs::remove_dir_all(root).ok();
}

#[test]
fn locale_strings_normalize_to_the_two_supported_languages() {
    assert_eq!(normalize_locale("zh-CN").as_deref(), Some("zh"));
    assert_eq!(normalize_locale("zh_CN.UTF-8").as_deref(), Some("zh"));
    assert_eq!(normalize_locale("ZH-Hans").as_deref(), Some("zh"));
    assert_eq!(normalize_locale("en-US").as_deref(), Some("en"));
    assert_eq!(normalize_locale("fr-FR").as_deref(), Some("en"));
    assert_eq!(normalize_locale("C").as_deref(), Some("en"));
    assert_eq!(normalize_locale(""), None);
    assert_eq!(normalize_locale("   "), None);
}

#[test]
fn write_project_config_round_trips_through_read_language() {
    let root = crate::test_support::unique_temp_dir("write-round-trip");

    let path = write_project_config(&root, "zh").unwrap();

    assert_eq!(path, project_config_path(&root));
    assert_eq!(read_language(&path).as_deref(), Some("zh"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn write_project_config_merges_instead_of_clobbering_other_keys() {
    let root = crate::test_support::unique_temp_dir("write-merge");
    let path = project_config_path(&root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, r#"{"otherSetting": "keep-me"}"#).unwrap();

    write_project_config(&root, "en").unwrap();

    let bytes = fs::read(&path).unwrap();
    let document: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(document["language"], "en");
    assert_eq!(document["otherSetting"], "keep-me");
    fs::remove_dir_all(root).ok();
}

#[test]
fn write_project_config_recovers_from_a_corrupt_existing_file() {
    let root = crate::test_support::unique_temp_dir("write-corrupt-recovery");
    let path = project_config_path(&root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "not valid json").unwrap();

    write_project_config(&root, "zh").unwrap();

    assert_eq!(read_language(&path).as_deref(), Some("zh"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn an_absent_config_file_is_neither_data_nor_an_error() {
    let root = crate::test_support::unique_temp_dir("absent-config");
    assert_eq!(read_language(&project_config_path(&root)), None);
    fs::remove_dir_all(root).ok();
}
