//! `boundary_dependency` and `layer_order`: the two authoritative rule kinds
//! `sentrux_adapter::AUTHORITATIVE_RULE_KINDS` has always named but this
//! engine never computed (the adapter fell back to its coarser
//! `sentrux_gate`/`sentrux_check` command-level completeness path instead).
//! Both reuse the exact `use crate::segment` resolution `rust_import_cycles`
//! already trusts for cycle detection: a `use crate::X` only becomes an edge
//! when `X.rs` or `X/mod.rs` exists under the referencing file's crate
//! `src/` root, so a re-exported type name or an unresolved path never
//! manufactures a phantom dependency.
//!
//! `.sentrux/rules.toml` never needs a general TOML parser for this --
//! `[[boundary]]`/`[[layer]]` are flat, single-line-array tables, so
//! `table_array` hand-rolls the same "read `.sentrux/rules.toml` by hand"
//! theory `rule_value`/`integer_rule` already use for `[constraints]`,
//! rather than adding a `toml` crate dependency for two array-of-tables.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::{crate_source_root, crate_use_segments, Violation, MAX_VIOLATION_TARGETS};

/// file path -> resolved target file paths named by its `use crate::` lines.
pub(crate) fn crate_edges(
    repo: &Path,
    rust_files: &BTreeSet<String>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for path in rust_files {
        let Some(source_root) = crate_source_root(path) else {
            continue;
        };
        let content = std::fs::read(repo.join(path)).unwrap_or_default();
        let content = String::from_utf8_lossy(&content);
        for segment in crate_use_segments(&content) {
            let module_file = format!("{source_root}/{segment}.rs");
            let module_directory = format!("{source_root}/{segment}/mod.rs");
            let target = if rust_files.contains(&module_file) {
                module_file
            } else if rust_files.contains(&module_directory) {
                module_directory
            } else {
                continue;
            };
            if &target != path {
                edges.entry(path.clone()).or_default().insert(target);
            }
        }
    }
    edges
}

/// The top-level crate module a file belongs to: the first path segment
/// under its crate's `src/` root, kept only when that segment resolves to a
/// real `X.rs`/`X/mod.rs` module file -- the same resolution `crate_edges`
/// uses, so a file nested under another module only via `#[path]` (never
/// declared `mod` at crate root) is left unclassified rather than guessed.
pub(crate) fn owning_module(path: &str, rust_files: &BTreeSet<String>) -> Option<String> {
    let source_root = crate_source_root(path)?;
    let relative = path.strip_prefix(&format!("{source_root}/"))?;
    match relative.split_once('/') {
        // Nested under a real subdirectory: the first segment is a bare
        // module name (no extension), matched the same way `crate_edges`
        // resolves a `use crate::` target.
        Some((first, _rest)) => {
            let module_file = format!("{source_root}/{first}.rs");
            let module_directory = format!("{source_root}/{first}/mod.rs");
            (rust_files.contains(&module_file) || rust_files.contains(&module_directory))
                .then(|| first.to_string())
        }
        // A flat file directly under `src/` already IS its own top-level
        // module file (it came from `rust_files`, so it exists); the module
        // name is its stem, not the filename with `.rs` appended again.
        None => relative.strip_suffix(".rs").map(str::to_string),
    }
}

pub(crate) struct BoundaryRule {
    pub(crate) description: String,
    pub(crate) from: Vec<String>,
    pub(crate) forbid: Vec<String>,
}

pub(crate) struct Layer {
    pub(crate) modules: Vec<String>,
}

pub(crate) fn parse_boundaries(rules: &str) -> Vec<BoundaryRule> {
    table_array(rules, "boundary")
        .into_iter()
        .map(|table| BoundaryRule {
            description: string_field(&table, "description"),
            from: array_field(&table, "from"),
            forbid: array_field(&table, "forbid"),
        })
        .filter(|rule| !rule.from.is_empty() && !rule.forbid.is_empty())
        .collect()
}

pub(crate) fn parse_layers(rules: &str) -> Vec<Layer> {
    table_array(rules, "layer")
        .into_iter()
        .map(|table| Layer {
            modules: array_field(&table, "modules"),
        })
        .filter(|layer| !layer.modules.is_empty())
        .collect()
}

/// `from`/`forbid` name an explicit module-to-module boundary that must
/// never be crossed, independent of any general layer ordering.
pub(crate) fn boundary_violations(
    rules: &[BoundaryRule],
    edges: &BTreeMap<String, BTreeSet<String>>,
    modules_by_file: &BTreeMap<String, String>,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    for rule in rules {
        let mut targets: Vec<String> = Vec::new();
        for (source, dests) in edges {
            let Some(source_module) = modules_by_file.get(source) else {
                continue;
            };
            if !rule.from.iter().any(|module| module == source_module) {
                continue;
            }
            for dest in dests {
                let Some(dest_module) = modules_by_file.get(dest) else {
                    continue;
                };
                if rule.forbid.iter().any(|module| module == dest_module) {
                    targets.push(format!("{source} -> {dest}"));
                }
            }
        }
        if targets.is_empty() {
            continue;
        }
        targets.sort();
        targets.dedup();
        targets.truncate(MAX_VIOLATION_TARGETS);
        violations.push(Violation {
            rule: "boundary_dependency".into(),
            message: format!(
                "boundary_dependency violated: {} ({:?} must not depend on {:?})",
                rule.description, rule.from, rule.forbid
            ),
            targets,
        });
    }
    violations
}

/// `layers` in dependency order: index 0 is the innermost/foundational
/// layer. A later layer may depend on an earlier one; an earlier layer
/// depending on a later one is the violation.
pub(crate) fn layer_violations(
    layers: &[Layer],
    edges: &BTreeMap<String, BTreeSet<String>>,
    modules_by_file: &BTreeMap<String, String>,
) -> Vec<Violation> {
    if layers.is_empty() {
        return Vec::new();
    }
    let mut index_of_module: BTreeMap<&str, usize> = BTreeMap::new();
    for (index, layer) in layers.iter().enumerate() {
        for module in &layer.modules {
            index_of_module.insert(module.as_str(), index);
        }
    }
    let mut targets: Vec<String> = Vec::new();
    for (source, dests) in edges {
        let Some(source_module) = modules_by_file.get(source) else {
            continue;
        };
        let Some(&source_index) = index_of_module.get(source_module.as_str()) else {
            continue;
        };
        for dest in dests {
            let Some(dest_module) = modules_by_file.get(dest) else {
                continue;
            };
            let Some(&dest_index) = index_of_module.get(dest_module.as_str()) else {
                continue;
            };
            if source_index < dest_index {
                targets.push(format!("{source} -> {dest}"));
            }
        }
    }
    if targets.is_empty() {
        return Vec::new();
    }
    targets.sort();
    targets.dedup();
    targets.truncate(MAX_VIOLATION_TARGETS);
    vec![Violation {
        rule: "layer_order".into(),
        message: "layer_order violated: a foundational layer depends on a layer declared above it"
            .into(),
        targets,
    }]
}

/// Minimal `[[header]]` array-of-tables reader: collects `key = value` lines
/// between one `[[header]]` marker and the next `[[...]]`/`[...]` header (or
/// end of file) into one table per occurrence. No nesting, no multi-line
/// arrays, no quoting beyond a plain `"..."` wrapper -- exactly what
/// `[[boundary]]`/`[[layer]]` need and nothing a real TOML document could
/// need that this file's own rules ever use.
fn table_array(rules: &str, header: &str) -> Vec<BTreeMap<String, String>> {
    let marker = format!("[[{header}]]");
    let mut tables = Vec::new();
    let mut current: Option<BTreeMap<String, String>> = None;
    for line in rules.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if trimmed == marker {
            tables.extend(current.take());
            current = Some(BTreeMap::new());
            continue;
        }
        if trimmed.starts_with('[') {
            tables.extend(current.take());
            continue;
        }
        let Some(table) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let value = value.split('#').next().unwrap_or("").trim().to_string();
        table.insert(key.trim().to_string(), value);
    }
    tables.extend(current.take());
    tables
}

fn string_field(table: &BTreeMap<String, String>, key: &str) -> String {
    table
        .get(key)
        .map(|value| value.trim_matches('"').to_string())
        .unwrap_or_default()
}

fn array_field(table: &BTreeMap<String, String>, key: &str) -> Vec<String> {
    table
        .get(key)
        .map(|raw| {
            raw.trim()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .map(|item| item.trim().trim_matches('"').to_string())
                .filter(|item| !item.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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
        // `this_repository_has_no_resolved_import_cycles`: run the real
        // engine against this crate's own tree, using whatever
        // `.sentrux/rules.toml` currently declares.
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let run = super::super::run_check(&repo)
            .expect("sentrux check should run against this repository");
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
}
