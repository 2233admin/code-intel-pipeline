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

#[test]
fn graph_consumes_file_gate_and_excludes_private_worktrees() {
    let repo = unique_temp_dir();
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::create_dir_all(repo.join(".claude/worktrees/garbage/src")).unwrap();
    fs::write(repo.join("src/main.rs"), "mod lib;\n").unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn live() {}\n").unwrap();
    fs::write(
        repo.join(".claude/worktrees/garbage/src/garbage.rs"),
        "pub fn garbage() {}\n",
    )
    .unwrap();

    let graph = build_graph(&repo, "zh", false).unwrap();
    let node_paths: Vec<&str> = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|node| node["path"].as_str())
        .collect();
    assert!(node_paths.contains(&"src/main.rs"));
    assert!(node_paths.contains(&"src/lib.rs"));
    assert!(!node_paths
        .iter()
        .any(|path| path.contains(".claude/worktrees")));

    let file_gate = &graph["file_gate"];
    assert_eq!(file_gate["schema"], "code-intel-file-gate.v1");
    assert!(file_gate["by_gate"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["gate"] == "default_path"));
    assert!(file_gate.get("decisions").is_none());

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

/// A scratch directory unique per (process, call), not merely per instant.
///
/// This module is `#[path]`-included into 12 separate integration-test
/// binaries and `cargo test` runs those binaries as parallel *processes*,
/// so up to 12 independent callers share this naming scheme at once. Naming
/// the directory from `SystemTime::now()` alone made the name a function of
/// nothing but the clock: any two processes observing the same instant get
/// the *identical* path, and whichever finishes first deletes the tree the
/// other is still reading. That is the mechanism behind
/// `switching_documentation_language_changes_only_the_language_field`
/// failing intermittently on `windows-latest` (issue #175) — a shared
/// mutable path with no owner.
///
/// The race itself was not reproduced locally: on this machine the clock
/// does advance between consecutive calls, so a same-instant collision is
/// rare rather than routine. The fix is therefore structural — the process
/// id makes cross-process collision impossible instead of improbable —
/// rather than a repair of an observed local failure. `NEXT_ID` does the
/// same for repeated calls inside one process. The timestamp is kept only
/// so leftover directories from a killed run stay sortable by age.
fn unique_temp_dir() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "code-intel-graph-test-{}-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    dir
}

/// Guards `NEXT_ID`: a burst tighter than the clock could plausibly resolve
/// must still produce distinct names.
///
/// Honest limitation — this test does **not** reproduce #175. Run against
/// the old clock-only implementation on this machine it still passes,
/// because the clock does advance between consecutive calls here. It guards
/// the in-process half of the property and would catch a regression on a
/// platform with a coarser clock; the cross-process half, which is what
/// #175 actually was, is pinned by the test below.
#[test]
fn scratch_directories_stay_unique_within_a_single_clock_tick() {
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..1000 {
        let dir = unique_temp_dir();
        assert!(
            seen.insert(dir.clone()),
            "unique_temp_dir returned a duplicate inside one burst: {}",
            dir.display()
        );
    }
}

/// The counter above only separates calls *within* one process. Cross-process
/// separation rests entirely on the pid being part of the name, so pin that
/// rather than leave it to be refactored away.
#[test]
fn scratch_directory_names_carry_the_process_id() {
    let dir = unique_temp_dir();
    let name = dir
        .file_name()
        .and_then(|name| name.to_str())
        .expect("scratch directory name");
    assert!(
        name.contains(&std::process::id().to_string()),
        "the name must carry the process id or parallel test binaries can collide: {name}"
    );
}
