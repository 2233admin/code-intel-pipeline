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

pub(crate) fn test_commands(
    repo: &std::path::Path,
    changed: &[String],
    tests: &[String],
    co_location_fallback: bool,
) -> (Vec<String>, Vec<String>) {
    let mut commands = BTreeSet::new();
    let mut limitations = Vec::new();
    rust_test_commands(
        repo,
        changed,
        tests,
        co_location_fallback,
        &mut commands,
        &mut limitations,
    );
    let python = tests
        .iter()
        .filter(|path| path.ends_with(".py"))
        .cloned()
        .collect::<Vec<_>>();
    if !python.is_empty() {
        let runner = if repo.join("uv.lock").is_file() {
            "uv run pytest"
        } else {
            limitations.push(
                "No uv.lock was found at the repository root; Python tests use the active interpreter."
                    .to_string(),
            );
            "python -m pytest"
        };
        commands.insert(format!(
            "{runner} {}",
            python
                .iter()
                .map(|path| shell_arg(path))
                .collect::<Vec<_>>()
                .join(" ")
        ));
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
        let package_manager = declared_package_manager(repo);
        let runner = match package_manager {
            Some("bun") => "bun test",
            Some("pnpm") => "pnpm test --",
            Some("yarn") => "yarn test",
            Some("npm") => "npm test --",
            _ if repo.join("bun.lock").is_file() || repo.join("bun.lockb").is_file() => "bun test",
            _ if repo.join("pnpm-lock.yaml").is_file() => "pnpm test --",
            _ if repo.join("yarn.lock").is_file() => "yarn test",
            _ => {
                if !repo.join("package-lock.json").is_file() {
                    limitations.push(
                        "No supported JavaScript package-manager declaration or lockfile was found at the repository root; npm is an advisory fallback."
                            .to_string(),
                    );
                }
                "npm test --"
            }
        };
        commands.insert(format!(
            "{runner} {}",
            javascript
                .iter()
                .map(|path| shell_arg(path))
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    if tests.iter().any(|path| path.ends_with(".go")) {
        commands.insert("go test ./...".to_string());
    }
    if tests.iter().any(|path| path.ends_with(".java")) {
        commands.insert("mvn test".to_string());
    }
    (commands.into_iter().collect(), limitations)
}

fn rust_test_commands(
    repo: &std::path::Path,
    changed: &[String],
    tests: &[String],
    co_location_fallback: bool,
    commands: &mut BTreeSet<String>,
    limitations: &mut Vec<String>,
) {
    if co_location_fallback && rust_source_test_commands(repo, changed, commands) {
        limitations.push(
            "No Rust test was reachable through imports; the command targets unit tests in the changed crate and uses a module-name filter for fast feedback. Run the package or workspace suite for completion."
                .to_string(),
        );
        return;
    }
    let mut manifests: BTreeMap<String, (BTreeSet<String>, bool)> = BTreeMap::new();
    let mut missing_manifest = false;
    for test in tests.iter().filter(|path| path.ends_with(".rs")) {
        let Some(manifest) = nearest_manifest(repo, test) else {
            missing_manifest = true;
            continue;
        };
        let target = cargo_integration_target(&manifest, test);
        let selection = manifests.entry(manifest).or_default();
        if let Some(target) = target {
            selection.0.insert(target);
        } else {
            selection.1 = true;
        }
    }
    for (manifest, (targets, broad)) in manifests {
        let mut command = format!("cargo test --manifest-path {}", shell_arg(&manifest));
        if broad {
            limitations.push(format!(
                "At least one Rust candidate under {manifest} is not a standard Cargo integration-test target; the command falls back to the package test suite."
            ));
        } else {
            for target in targets {
                command.push_str(&format!(" --test {}", shell_arg(&target)));
            }
        }
        commands.insert(command);
    }
    if missing_manifest {
        commands.insert("cargo test".to_string());
        limitations.push(
            "At least one Rust candidate has no discoverable Cargo.toml; cargo test from the repository root is an advisory fallback."
                .to_string(),
        );
    }
}

fn rust_source_test_commands(
    repo: &std::path::Path,
    changed: &[String],
    commands: &mut BTreeSet<String>,
) -> bool {
    let mut added = false;
    for source in changed.iter().filter(|path| path.ends_with(".rs")) {
        let Some(manifest) = nearest_manifest(repo, source) else {
            continue;
        };
        let manifest_dir = std::path::Path::new(&manifest)
            .parent()
            .unwrap_or(std::path::Path::new(""));
        let Ok(relative) = std::path::Path::new(source).strip_prefix(manifest_dir) else {
            continue;
        };
        if !relative.starts_with("src") {
            continue;
        }
        let crate_root = repo.join(manifest_dir);
        let mut command = format!("cargo test --manifest-path {}", shell_arg(&manifest));
        if crate_root.join("src/lib.rs").is_file() {
            command.push_str(" --lib");
        }
        if crate_root.join("src/main.rs").is_file() {
            command.push_str(" --bins");
        }
        if !command.contains(" --lib") && !command.contains(" --bins") {
            continue;
        }
        let stem = relative.file_stem().and_then(|stem| stem.to_str());
        let filter = match stem {
            Some("lib" | "main") | None => None,
            Some("mod") => relative
                .parent()
                .and_then(std::path::Path::file_name)
                .and_then(|name| name.to_str()),
            other => other,
        };
        if let Some(filter) = filter {
            command.push(' ');
            command.push_str(&shell_arg(filter));
        }
        commands.insert(command);
        added = true;
    }
    added
}

fn nearest_manifest(repo: &std::path::Path, test: &str) -> Option<String> {
    let mut directory = repo.join(test).parent()?.to_path_buf();
    loop {
        let manifest = directory.join("Cargo.toml");
        if manifest.is_file() {
            return manifest
                .strip_prefix(repo)
                .ok()
                .map(|path| path.to_string_lossy().replace('\\', "/"));
        }
        if directory == repo || !directory.pop() || !directory.starts_with(repo) {
            return None;
        }
    }
}

fn cargo_integration_target(manifest: &str, test: &str) -> Option<String> {
    let manifest_dir = std::path::Path::new(manifest)
        .parent()
        .unwrap_or(std::path::Path::new(""));
    let relative = std::path::Path::new(test).strip_prefix(manifest_dir).ok()?;
    let parts = relative
        .components()
        .map(|part| part.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()?;
    match parts.as_slice() {
        ["tests", file] if file.ends_with(".rs") => std::path::Path::new(file)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::to_string),
        ["tests", directory, "main.rs"] => Some((*directory).to_string()),
        _ => None,
    }
}

fn declared_package_manager(repo: &std::path::Path) -> Option<&'static str> {
    let manifest = std::fs::read_to_string(repo.join("package.json")).ok()?;
    let value: Value = serde_json::from_str(&manifest).ok()?;
    let declared = value["packageManager"].as_str()?;
    ["bun", "pnpm", "yarn", "npm"]
        .into_iter()
        .find(|manager| declared == *manager || declared.starts_with(&format!("{manager}@")))
}

fn shell_arg(value: &str) -> String {
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "_+-./:".contains(character))
    {
        value.to_string()
    } else if cfg!(windows) {
        format!("'{}'", value.replace('\'', "''"))
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_repo(name: &str) -> std::path::PathBuf {
        let repo =
            std::env::temp_dir().join(format!("code-intel-impact-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        repo
    }

    #[test]
    fn rust_commands_target_the_owning_cargo_integration_tests() {
        let repo = temp_repo("rust-targets");
        std::fs::create_dir_all(repo.join("crates/app/tests")).unwrap();
        std::fs::write(repo.join("crates/app/Cargo.toml"), "[package]\n").unwrap();

        let (commands, limitations) = test_commands(
            &repo,
            &["crates/app/src/lib.rs".to_string()],
            &[
                "crates/app/tests/api.rs".to_string(),
                "crates/app/tests/cli/main.rs".to_string(),
            ],
            false,
        );

        assert_eq!(
            commands,
            ["cargo test --manifest-path crates/app/Cargo.toml --test api --test cli"]
        );
        assert!(limitations.is_empty());
        assert_eq!(shell_arg("tests/a & b.rs"), "'tests/a & b.rs'");
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn rust_co_location_fallback_uses_the_changed_source_target() {
        let repo = temp_repo("rust-source-fallback");
        std::fs::create_dir_all(repo.join("crates/app/src")).unwrap();
        std::fs::write(repo.join("crates/app/Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(repo.join("crates/app/src/main.rs"), "fn main() {}\n").unwrap();

        let (commands, limitations) = test_commands(
            &repo,
            &["crates/app/src/impact_graph.rs".to_string()],
            &["crates/app/tests/api.rs".to_string()],
            true,
        );

        assert_eq!(
            commands,
            ["cargo test --manifest-path crates/app/Cargo.toml --bins impact_graph"]
        );
        assert!(limitations[0].contains("fast feedback"));
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn repository_evidence_selects_python_and_typescript_runners() {
        let repo = temp_repo("language-runners");
        std::fs::write(repo.join("uv.lock"), "").unwrap();
        std::fs::write(
            repo.join("package.json"),
            r#"{"packageManager":"pnpm@10.0.0"}"#,
        )
        .unwrap();

        let (commands, limitations) = test_commands(
            &repo,
            &[],
            &[
                "tests/test_api.py".to_string(),
                "src/app.test.ts".to_string(),
            ],
            false,
        );

        assert_eq!(
            commands,
            [
                "pnpm test -- src/app.test.ts",
                "uv run pytest tests/test_api.py"
            ]
        );
        assert!(limitations.is_empty());
        let _ = std::fs::remove_dir_all(repo);
    }
}
