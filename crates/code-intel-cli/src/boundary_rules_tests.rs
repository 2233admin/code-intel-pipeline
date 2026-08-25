use super::*;
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn table_array_reads_one_table_per_marker_and_stops_at_the_next_header() {
    let rules = r#"
[constraints]
max_cc = 156

[[boundary]]
description = "engine must not depend on cli"
from = ["sentrux_gate", "sentrux_analysis"]
forbid = ["cli"]

[[boundary]]
description = "second rule"
from = ["a"]
forbid = ["b", "c"]

[[layer]]
modules = ["sentrux_gate"]
"#;
    let boundaries = parse_boundaries(rules);
    assert_eq!(boundaries.len(), 2);
    assert_eq!(boundaries[0].description, "engine must not depend on cli");
    assert_eq!(boundaries[0].from, vec!["sentrux_gate", "sentrux_analysis"]);
    assert_eq!(boundaries[0].forbid, vec!["cli"]);
    assert_eq!(boundaries[1].forbid, vec!["b", "c"]);

    let layers = parse_layers(rules);
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0].modules, vec!["sentrux_gate"]);
}

#[test]
fn owning_module_resolves_nested_files_to_their_top_level_module() {
    let rust_files: BTreeSet<String> = [
        "crates/x/src/cli/mod.rs",
        "crates/x/src/cli/legacy.rs",
        "crates/x/src/sentrux_gate.rs",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert_eq!(
        owning_module("crates/x/src/cli/legacy.rs", &rust_files),
        Some("cli".to_string())
    );
    assert_eq!(
        owning_module("crates/x/src/sentrux_gate.rs", &rust_files),
        Some("sentrux_gate".to_string())
    );
    // A nested file whose first path segment names neither `<segment>.rs`
    // nor `<segment>/mod.rs` in the tree resolves to no real top-level
    // crate module, so it is left unclassified rather than guessed.
    assert_eq!(
        owning_module("crates/x/src/ghost/deep.rs", &rust_files),
        None
    );
}

#[test]
fn boundary_violations_only_fire_for_the_declared_direction() {
    let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    edges
        .entry("crates/x/src/sentrux_gate.rs".into())
        .or_default()
        .insert("crates/x/src/cli/mod.rs".into());
    let mut modules_by_file = BTreeMap::new();
    modules_by_file.insert(
        "crates/x/src/sentrux_gate.rs".to_string(),
        "sentrux_gate".to_string(),
    );
    modules_by_file.insert("crates/x/src/cli/mod.rs".to_string(), "cli".to_string());

    let rules = vec![BoundaryRule {
        description: "core must not depend on cli".into(),
        from: vec!["sentrux_gate".into()],
        forbid: vec!["cli".into()],
    }];
    let violations = boundary_violations(&rules, &edges, &modules_by_file);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].rule, "boundary_dependency");
    assert_eq!(
        violations[0].targets,
        vec!["crates/x/src/sentrux_gate.rs -> crates/x/src/cli/mod.rs"]
    );

    // The reverse direction (cli -> sentrux_gate) is exactly what's
    // expected of an outer layer and must never trip this rule.
    let mut reverse_edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    reverse_edges
        .entry("crates/x/src/cli/mod.rs".into())
        .or_default()
        .insert("crates/x/src/sentrux_gate.rs".into());
    assert!(boundary_violations(&rules, &reverse_edges, &modules_by_file).is_empty());
}

#[test]
fn layer_violations_only_fire_when_an_earlier_layer_depends_on_a_later_one() {
    let layers = vec![
        Layer {
            modules: vec!["core".into()],
        },
        Layer {
            modules: vec!["cli".into()],
        },
    ];
    let mut modules_by_file = BTreeMap::new();
    modules_by_file.insert("src/core.rs".to_string(), "core".to_string());
    modules_by_file.insert("src/cli.rs".to_string(), "cli".to_string());

    let mut forward_edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    forward_edges
        .entry("src/cli.rs".into())
        .or_default()
        .insert("src/core.rs".into());
    assert!(layer_violations(&layers, &forward_edges, &modules_by_file).is_empty());

    let mut backward_edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    backward_edges
        .entry("src/core.rs".into())
        .or_default()
        .insert("src/cli.rs".into());
    let violations = layer_violations(&layers, &backward_edges, &modules_by_file);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].rule, "layer_order");
    assert_eq!(violations[0].targets, vec!["src/core.rs -> src/cli.rs"]);
}

#[test]
fn this_repository_declares_no_boundary_or_layer_violation() {
    // Same self-check convention as
    // `this_repository_has_no_resolved_import_cycles`: run the real engine
    // against this crate's own tree, using whatever `.sentrux/rules.toml`
    // currently declares. This proves the *declared* rules hold; it cannot
    // prove the detection machinery itself still works if `crate_edges`
    // regressed to always returning nothing -- that's what the fixture test
    // below is for.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let run = super::super::expect_check_ran(&repo);
    let violations: Vec<&Violation> = run
        .violations
        .iter()
        .filter(|violation| {
            violation.rule == "boundary_dependency" || violation.rule == "layer_order"
        })
        .collect();
    assert!(
        violations.is_empty(),
        "boundary_dependency/layer_order violation(s) detected: {violations:?}"
    );
}

fn fixture_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "boundary-rules-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src")).expect("create fixture src/");
    fs::create_dir_all(root.join(".sentrux")).expect("create fixture .sentrux/");
    root
}

/// The self-check above only proves this repository's own tree has no
/// violation today -- a `crate_edges`/`owning_module` regression that always
/// returns nothing would pass it just as well. This drives `run_check` end
/// to end against a real two-file fixture with a real `use crate::cli`
/// edge, so a regression in the detection path itself fails a plain
/// `cargo test`.
#[test]
fn run_check_detects_a_real_boundary_and_layer_violation_end_to_end() {
    let repo = fixture_root("violation");
    fs::write(
        repo.join("src").join("core.rs"),
        "use crate::cli;\n\npub fn touch() {\n    cli::noop();\n}\n",
    )
    .expect("write core.rs");
    fs::write(repo.join("src").join("cli.rs"), "pub fn noop() {}\n").expect("write cli.rs");
    fs::write(
        repo.join(".sentrux").join("rules.toml"),
        "[[layer]]\nmodules = [\"core\"]\n\n[[layer]]\nmodules = [\"cli\"]\n\n[[boundary]]\ndescription = \"core must not depend on cli\"\nfrom = [\"core\"]\nforbid = [\"cli\"]\n",
    )
    .expect("write rules.toml");

    let run = super::super::expect_check_ran(&repo);
    assert!(
        !run.success,
        "the fixture's core -> cli edge must fail check"
    );

    let boundary = run
        .violations
        .iter()
        .find(|violation| violation.rule == "boundary_dependency")
        .expect("boundary_dependency violation expected");
    assert_eq!(boundary.targets, vec!["src/core.rs -> src/cli.rs"]);

    let layer = run
        .violations
        .iter()
        .find(|violation| violation.rule == "layer_order")
        .expect("layer_order violation expected");
    assert_eq!(layer.targets, vec!["src/core.rs -> src/cli.rs"]);

    fs::remove_dir_all(&repo).ok();
}

/// Complement to the violation fixture above: two files that both declare
/// the same layer/boundary config but never cross it must stay green,
/// proving the checks don't fire on every edge, only the forbidden one.
#[test]
fn run_check_passes_a_fixture_with_no_forbidden_edge() {
    let repo = fixture_root("clean");
    fs::write(repo.join("src").join("core.rs"), "pub fn touch() {}\n").expect("write core.rs");
    fs::write(
        repo.join("src").join("cli.rs"),
        "use crate::core;\n\npub fn noop() {\n    core::touch();\n}\n",
    )
    .expect("write cli.rs");
    fs::write(
        repo.join(".sentrux").join("rules.toml"),
        "[[layer]]\nmodules = [\"core\"]\n\n[[layer]]\nmodules = [\"cli\"]\n\n[[boundary]]\ndescription = \"core must not depend on cli\"\nfrom = [\"core\"]\nforbid = [\"cli\"]\n",
    )
    .expect("write rules.toml");

    let run = super::super::expect_check_ran(&repo);
    let violations: Vec<&Violation> = run
        .violations
        .iter()
        .filter(|violation| {
            violation.rule == "boundary_dependency" || violation.rule == "layer_order"
        })
        .collect();
    assert!(
        violations.is_empty(),
        "cli -> core is the expected direction and must never violate: {violations:?}"
    );

    fs::remove_dir_all(&repo).ok();
}
