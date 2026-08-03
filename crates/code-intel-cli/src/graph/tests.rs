use super::*;

#[test]
fn builds_graph_from_local_sources() {
    let repo = unique_temp_dir();
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(
        repo.join("src").join("lib.rs"),
        "mod graph;\npub fn run() {}\n",
    )
    .unwrap();
    fs::write(repo.join("src").join("graph.rs"), "pub struct Node;\n").unwrap();

    let graph = build_graph(&repo, "zh", false).unwrap();

    assert_eq!(graph["summary"]["files"].as_u64(), Some(2));
    assert!(graph["summary"]["edges"].as_u64().unwrap_or(0) >= 1);
    assert!(graph["summary"]["symbols"].as_u64().unwrap_or(0) >= 2);

    // This document is embedded verbatim in the content-addressed
    // `observed.evidence.payload`, so no field may carry a wall clock:
    // that hands an unchanged tree a new payload digest every run.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    for (key, value) in graph.as_object().unwrap() {
        assert!(
            !value
                .as_u64()
                .is_some_and(|seconds| seconds.abs_diff(now) < 86_400),
            "graph document field {key} carries a wall clock"
        );
    }

    fs::remove_dir_all(repo).unwrap();
}

/// Issue #155 acceptance: switching the resolved documentation language
/// must never change machine-readable shape. `language` is the only
/// field this document lets vary with the setting (issue #101: schema,
/// `kind`/`language`-as-a-detected-source-language-tag on nodes, and
/// every other field are machine-first and language-invariant) -- so a
/// zh run and an en run must be byte-identical everywhere else.
#[test]
fn switching_documentation_language_changes_only_the_language_field() {
    let repo = unique_temp_dir();
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(
        repo.join("src").join("lib.rs"),
        "mod graph;\npub fn run() {}\n",
    )
    .unwrap();
    fs::write(repo.join("src").join("graph.rs"), "pub struct Node;\n").unwrap();

    let mut zh = build_graph(&repo, "zh", true).unwrap();
    let mut en = build_graph(&repo, "en", true).unwrap();

    assert_eq!(zh["language"], "zh");
    assert_eq!(en["language"], "en");
    zh["language"] = Value::Null;
    en["language"] = Value::Null;
    assert_eq!(
        zh, en,
        "graph JSON must be identical outside the language field itself"
    );

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn truncates_unicode_on_a_character_boundary() {
    let value = "交易账户与行情连接是两个独立概念";

    let truncated = truncate(value, 10);

    assert_eq!(truncated, "交易账...");
}

fn unique_temp_dir() -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};

    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "code-intel-graph-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    dir
}
