//! `code-intel edit impact` — blast radius for files an agent is editing
//! right now, computed from the working tree.
//!
//! This is deliberately NOT `change impact`. That route answers the same
//! question authoritatively: it binds to a committed run, verifies every
//! artifact digest, and refuses when the snapshot is stale. Those guarantees
//! cost a full `run execute` beforehand plus a snapshot identity on every
//! call, which is why nothing reached for it mid-edit — issue #58's finding
//! was that every agent-facing surface is read-shaped or a post-hoc gate.
//!
//! The observation this route exploits: the computation needs a file list and
//! an import graph, nothing else. The snapshot/artifact-root/digest machinery
//! exists to make the answer *authoritative*, not to make it *computable*. So
//! this route drops authority and keeps the computation, and says so in the
//! payload: `"authority":"none"`. A gate that consumed this would be lying,
//! and `edit_impact_output_can_never_be_mistaken_for_authoritative` asserts
//! the schema name never appears in an admissibility or gate path.
//!
//! What it must never become is a second implementation. The traversal is
//! `impact_graph`, shared verbatim with `change impact`; the import
//! heuristics are the ones `evidence.native-code` already publishes. If the
//! two ever disagree, an agent's mid-edit answer would contradict the gate's
//! answer about the same tree, which is worse than having no mid-edit answer.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::capability_inventory::native_code_evidence;
use crate::impact_graph::{impacted_files, reverse_import_graph, select_tests, test_commands};

#[path = "hardened_git.rs"]
mod hardened_git;

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    // Leaf adapter only — WorkspaceAdvisoryController wraps `execute` for the
    // typed authority surface and must not be imported here (import cycle).
    match EditImpactRequest::parse(raw).and_then(execute) {
        Ok(result) => {
            println!("{}", serde_json::to_string(result.value()).unwrap());
            0
        }
        Err(EditImpactError::Contract(message)) => {
            eprintln!("{message}");
            65
        }
        Err(EditImpactError::HostIo(message)) => {
            eprintln!("{message}");
            74
        }
    }
}

#[derive(Debug)]
pub(crate) enum EditImpactError {
    Contract(String),
    HostIo(String),
}

pub(crate) struct EditImpactRequest {
    pub(crate) repo_path: PathBuf,
    changed: Vec<String>,
    scopes: Vec<String>,
}

impl EditImpactRequest {
    pub(crate) fn parse(raw: &[String]) -> Result<Self, EditImpactError> {
        let mut repo_path: Option<PathBuf> = None;
        let mut changed = Vec::new();
        let mut scopes = Vec::new();
        let mut index = 0;
        while index < raw.len() {
            let token = raw[index].as_str();
            let value = raw.get(index + 1).filter(|value| !value.starts_with("--"));
            index += match (token, value) {
                ("--repo-path", Some(value)) => {
                    repo_path = Some(PathBuf::from(value));
                    2
                }
                ("--changed", Some(value)) => {
                    changed.push(normalize_relative(value)?);
                    2
                }
                ("--scope", Some(value)) => {
                    scopes.push(normalize_relative(value)?);
                    2
                }
                ("--repo-path" | "--changed" | "--scope", None) => {
                    return Err(EditImpactError::Contract(format!(
                        "{token} requires a value"
                    )))
                }
                (other, _) => {
                    return Err(EditImpactError::Contract(format!(
                        "unknown argument for edit impact: {other}"
                    )))
                }
            };
        }
        let repo_path = repo_path
            .ok_or_else(|| EditImpactError::Contract("edit impact requires --repo-path".into()))?;
        if changed.is_empty() {
            return Err(EditImpactError::Contract(
                "edit impact requires at least one --changed <path>".into(),
            ));
        }
        Ok(Self {
            repo_path,
            changed,
            scopes,
        })
    }
}

pub(crate) struct EditImpactResult {
    value: Value,
    repo_root: PathBuf,
}

impl EditImpactResult {
    pub(crate) fn value(&self) -> &Value {
        &self.value
    }

    pub(crate) fn repo_root(&self) -> &Path {
        &self.repo_root
    }
}

pub(crate) fn execute(cli: EditImpactRequest) -> Result<EditImpactResult, EditImpactError> {
    let repo = cli.repo_path.canonicalize().map_err(|error| {
        EditImpactError::Contract(format!(
            "cannot resolve --repo-path {}: {error}",
            cli.repo_path.display()
        ))
    })?;
    let inventory = working_tree_files(&repo, &cli.scopes)?;
    let files = inventory.iter().cloned().collect::<BTreeSet<_>>();

    let mut imports = Vec::new();
    let mut unreadable = Vec::new();
    let mut unsupported = BTreeSet::new();
    for path in &inventory {
        let language = native_code_evidence::language(path);
        if language == "unknown" {
            continue;
        }
        let Ok(bytes) = std::fs::read(repo.join(path)) else {
            unreadable.push(path.clone());
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        let lines = text.lines().collect::<Vec<_>>();
        let extracted = native_code_evidence::extract_imports(path, language, &lines);
        if extracted.is_empty() {
            unsupported.insert(language.to_string());
        }
        imports.extend(extracted);
    }

    let (reverse, resolved, unresolved) =
        reverse_import_graph(&imports, &files).map_err(EditImpactError::Contract)?;
    let impacted = impacted_files(&cli.changed, &files, &reverse);
    let tests = select_tests(&impacted, &cli.changed, &files);
    let commands = test_commands(&tests);

    let changed_rows = cli
        .changed
        .iter()
        .map(|path| json!({"path":path,"inInventory":files.contains(path)}))
        .collect::<Vec<_>>();
    let impact_rows = impacted
        .iter()
        .map(|(path, reason)| {
            json!({
                "path":path,
                "distance":reason.distance,
                "reason":reason.reason,
                "via":reason.via,
                "confidence":reason.confidence,
            })
        })
        .collect::<Vec<_>>();

    let mut limitations = vec![
        json!("No authoritative run backs this answer. It is derived from the working tree, is not admissible evidence, and must never gate."),
        json!("Native import extraction is heuristic and does not prove runtime call paths."),
        json!("Dynamic imports, generated code, reflection, build-system edges, and external packages may be unresolved."),
        json!("Test commands are candidates only and are never executed by this command."),
    ];
    if !unreadable.is_empty() {
        limitations.push(json!(format!(
            "{} file(s) could not be read and contribute no edges.",
            unreadable.len()
        )));
    }

    let repo_path = repo.display().to_string();
    Ok(EditImpactResult {
        repo_root: repo,
        value: json!({
            "schema":"code-intel-edit-impact.v1",
            "authority":"none",
            "source":"working-tree",
            "repoPath":repo_path,
            "scopes":cli.scopes,
            "inventoryFiles":inventory.len(),
            "changed":changed_rows,
            "impact":{
                "files":impact_rows,
                "resolvedImportEdges":resolved,
                "unresolvedImportEdges":unresolved,
            },
            "testSelection":{
                "status":if tests.is_empty() { "none" } else { "candidates" },
                "files":tests,
                "commands":commands,
                "advisoryOnly":true,
                "rationale":"Select impacted test files reachable through the working-tree reverse import graph; use same-module test co-location only as a fallback.",
            },
            // Named rather than inferred from an empty result: an agent that
            // cannot tell "no impact" from "this language is not modelled" learns
            // the tool is useless and stops calling it.
            "languagesWithoutExtractedImports":unsupported.into_iter().collect::<Vec<_>>(),
            "limitations":limitations,
        }),
    })
}

/// Tracked files plus untracked-but-not-ignored ones, in a single Git call.
///
/// Mid-edit, a file the agent created seconds ago matters as much as a
/// tracked one, and a walk that ignored `.gitignore` would pull in `target/`
/// and blow the latency budget. `--cached --others --exclude-standard` is
/// exactly that set.
fn working_tree_files(repo: &Path, scopes: &[String]) -> Result<Vec<String>, EditImpactError> {
    let output = hardened_git::command(repo)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()
        .map_err(|error| EditImpactError::HostIo(format!("cannot launch Git: {error}")))?;
    if !output.status.success() {
        return Err(EditImpactError::HostIo(format!(
            "cannot list working tree files: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let mut files = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8_lossy(entry).replace('\\', "/"))
        .filter(|path| scopes.is_empty() || scopes.iter().any(|scope| in_scope(path, scope)))
        .collect::<Vec<_>>();
    files.sort();
    files.dedup();
    Ok(files)
}

fn in_scope(path: &str, scope: &str) -> bool {
    path == scope || path.starts_with(&format!("{scope}/"))
}

fn normalize_relative(path: &str) -> Result<String, EditImpactError> {
    let path = path.replace('\\', "/");
    if path.is_empty()
        || path.starts_with('/')
        || path.split('/').any(|segment| segment == "..")
        || path.contains(':')
    {
        return Err(EditImpactError::Contract(format!(
            "paths must be portable and repository-relative: {path}"
        )));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_must_be_portable_and_repository_relative() {
        assert!(normalize_relative("src/a.rs").is_ok());
        assert_eq!(normalize_relative("src\\a.rs").unwrap(), "src/a.rs");
        for bad in ["", "/abs/a.rs", "../escape.rs", "C:/x.rs"] {
            assert!(normalize_relative(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn edit_impact_output_can_never_be_mistaken_for_authoritative() {
        // The payload says authority:none, but a reader can ignore a string.
        // This asserts the stronger property: no admission, gate, or index
        // path in this crate mentions the schema at all, so there is nothing
        // for a future change to accidentally start consuming.
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for module in [
            "admissibility.rs",
            "sentrux_gate.rs",
            "artifact_index.rs",
            "artifact_ref.rs",
            "committed_evidence.rs",
        ] {
            let text = std::fs::read_to_string(src.join(module))
                .unwrap_or_else(|error| panic!("read {module}: {error}"));
            assert!(
                !text.contains("code-intel-edit-impact"),
                "{module} references the non-authoritative edit-impact schema"
            );
        }
    }

    #[test]
    fn scope_matches_a_directory_prefix_not_a_string_prefix() {
        assert!(in_scope("crates/cli/main.rs", "crates"));
        assert!(in_scope("crates", "crates"));
        // "crates-old" must not match the "crates" scope.
        assert!(!in_scope("crates-old/main.rs", "crates"));
    }
}
