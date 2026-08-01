//! Reverse-import impact graph, shared by the authoritative `change impact`
//! route and the working-tree `edit impact` route.
//!
//! These functions were always independent of the evidence layer — they take
//! a file set and a list of import records and return reachability. Keeping
//! them inside `change_impact` would have forced the working-tree route to
//! either restate the traversal or inherit the committed-run prerequisite it
//! exists to avoid.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde_json::Value;

#[derive(Clone)]
pub(crate) struct ReverseEdge {
    pub(crate) importer: String,
    pub(crate) confidence: &'static str,
}

pub(crate) fn reverse_import_graph(
    imports: &[Value],
    files: &BTreeSet<String>,
) -> Result<(BTreeMap<String, Vec<ReverseEdge>>, usize, usize), String> {
    let mut reverse: BTreeMap<String, Vec<ReverseEdge>> = BTreeMap::new();
    let mut resolved = 0;
    let mut unresolved = 0;
    for import in imports {
        let (Some(importer), Some(target)) = (import["file"].as_str(), import["target"].as_str())
        else {
            return Err(
                "code_evidence.imports entries must carry string file and target fields"
                    .to_string(),
            );
        };
        if let Some((target, confidence)) = resolve_import(importer, target, files) {
            reverse.entry(target).or_default().push(ReverseEdge {
                importer: importer.to_string(),
                confidence,
            });
            resolved += 1;
        } else {
            unresolved += 1;
        }
    }
    for edges in reverse.values_mut() {
        edges.sort_by(|left, right| left.importer.cmp(&right.importer));
        edges.dedup_by(|left, right| left.importer == right.importer);
    }
    Ok((reverse, resolved, unresolved))
}

fn resolve_import(
    importer: &str,
    target: &str,
    files: &BTreeSet<String>,
) -> Option<(String, &'static str)> {
    let target = target.replace('\\', "/");
    let mut candidates = Vec::new();
    if target.starts_with('.') {
        let parent = importer.rsplit_once('/').map(|pair| pair.0).unwrap_or("");
        candidates.push(join_relative(parent, &target)?);
    } else if let Some(rest) = target.strip_prefix("crate::") {
        candidates.push(format!("src/{}", rest.replace("::", "/")));
    } else {
        candidates.push(target.replace("::", "/").replace('.', "/"));
    }
    for base in &candidates {
        for candidate in path_candidates(base) {
            if files.contains(&candidate) {
                return Some((candidate, "high"));
            }
        }
    }
    let token = candidates.last()?.trim_matches('/');
    let suffixes = files
        .iter()
        .filter(|path| {
            let without_extension = path.rsplit_once('.').map(|pair| pair.0).unwrap_or(path);
            without_extension == token || without_extension.ends_with(&format!("/{token}"))
        })
        .cloned()
        .collect::<Vec<_>>();
    match suffixes.as_slice() {
        [only] => Some((only.clone(), "medium")),
        _ => None,
    }
}

fn join_relative(parent: &str, target: &str) -> Option<String> {
    let mut components = parent
        .split('/')
        .filter(|component| !component.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    for component in target.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            value => components.push(value.to_string()),
        }
    }
    Some(components.join("/"))
}

fn path_candidates(base: &str) -> Vec<String> {
    let mut values = vec![base.to_string()];
    if base
        .rsplit('/')
        .next()
        .is_some_and(|name| !name.contains('.'))
    {
        for extension in ["rs", "py", "js", "jsx", "ts", "tsx", "go", "java"] {
            values.push(format!("{base}.{extension}"));
            values.push(format!("{base}/index.{extension}"));
        }
        values.push(format!("{base}/mod.rs"));
        values.push(format!("{base}/__init__.py"));
    }
    values
}

pub(crate) struct ImpactReason {
    pub(crate) distance: usize,
    pub(crate) reason: &'static str,
    pub(crate) via: Option<String>,
    pub(crate) confidence: &'static str,
}

pub(crate) fn impacted_files(
    changed: &[String],
    files: &BTreeSet<String>,
    reverse: &BTreeMap<String, Vec<ReverseEdge>>,
) -> BTreeMap<String, ImpactReason> {
    let mut impacted = BTreeMap::new();
    let mut queue = VecDeque::new();
    for path in changed {
        if files.contains(path) {
            impacted.insert(
                path.clone(),
                ImpactReason {
                    distance: 0,
                    reason: "changed",
                    via: None,
                    confidence: "high",
                },
            );
            queue.push_back(path.clone());
        }
    }
    while let Some(target) = queue.pop_front() {
        let distance = impacted[&target].distance + 1;
        for edge in reverse.get(&target).into_iter().flatten() {
            if impacted.contains_key(&edge.importer) {
                continue;
            }
            impacted.insert(
                edge.importer.clone(),
                ImpactReason {
                    distance,
                    reason: "reverse_import",
                    via: Some(target.clone()),
                    confidence: edge.confidence,
                },
            );
            queue.push_back(edge.importer.clone());
        }
    }
    impacted
}

pub(crate) fn select_tests(
    impacted: &BTreeMap<String, ImpactReason>,
    changed: &[String],
    files: &BTreeSet<String>,
) -> Vec<String> {
    let mut tests = impacted
        .keys()
        .filter(|path| test_file(path))
        .cloned()
        .collect::<BTreeSet<_>>();
    if tests.is_empty() {
        let modules = changed
            .iter()
            .filter_map(|path| path.split('/').next())
            .collect::<BTreeSet<_>>();
        tests.extend(
            files
                .iter()
                .filter(|path| {
                    test_file(path)
                        && path
                            .split('/')
                            .next()
                            .is_some_and(|module| modules.contains(module))
                })
                .cloned(),
        );
    }
    tests.into_iter().collect()
}

pub(crate) fn test_commands(tests: &[String]) -> Vec<String> {
    let mut commands = BTreeSet::new();
    if tests.iter().any(|path| path.ends_with(".rs")) {
        commands.insert("cargo test".to_string());
    }
    let python = tests
        .iter()
        .filter(|path| path.ends_with(".py"))
        .cloned()
        .collect::<Vec<_>>();
    if !python.is_empty() {
        commands.insert(format!("pytest {}", python.join(" ")));
    }
    let javascript = tests
        .iter()
        .filter(|path| {
            [".js", ".jsx", ".ts", ".tsx"]
                .iter()
                .any(|extension| path.ends_with(extension))
        })
        .cloned()
        .collect::<Vec<_>>();
    if !javascript.is_empty() {
        commands.insert(format!("npm test -- {}", javascript.join(" ")));
    }
    if tests.iter().any(|path| path.ends_with(".go")) {
        commands.insert("go test ./...".to_string());
    }
    if tests.iter().any(|path| path.ends_with(".java")) {
        commands.insert("mvn test".to_string());
    }
    commands.into_iter().collect()
}

pub(crate) fn test_file(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.contains("test.")
        || file.contains("_test.")
        || file.contains("spec.")
        || path.starts_with("test/")
        || path.starts_with("tests/")
        || path.starts_with("spec/")
        || path.contains("/test/")
        || path.contains("/tests/")
        || path.contains("/spec/")
}
