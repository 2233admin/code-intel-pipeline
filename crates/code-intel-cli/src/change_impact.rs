use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::committed_evidence::{self, EvidenceError};

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    match Cli::parse(raw).and_then(execute) {
        Ok(result) => {
            println!("{}", serde_json::to_string(&result).unwrap());
            0
        }
        Err(ImpactError::Contract(message)) => {
            eprintln!("{message}");
            65
        }
        Err(ImpactError::HostIo(message)) => {
            eprintln!("{message}");
            74
        }
    }
}

struct Cli {
    artifact_root: PathBuf,
    repo: String,
    repo_path: PathBuf,
    changed: Vec<String>,
}

impl Cli {
    fn parse(raw: &[String]) -> Result<Self, ImpactError> {
        if raw.first().map(String::as_str) != Some("impact") {
            return Err(ImpactError::Contract("usage: change impact --artifact-root <root> --repo <name> --repo-path <checkout> --changed <relative-path> [--changed <relative-path>]...".into()));
        }
        let mut artifact_root = None;
        let mut repo = None;
        let mut repo_path = None;
        let mut changed = Vec::new();
        let mut index = 1;
        while index < raw.len() {
            let flag = raw[index].as_str();
            if !matches!(
                flag,
                "--artifact-root" | "--repo" | "--repo-path" | "--changed"
            ) {
                return Err(ImpactError::Contract(format!(
                    "unknown change impact argument: {flag}"
                )));
            }
            let value = raw
                .get(index + 1)
                .filter(|value| !value.is_empty() && !value.starts_with("--"))
                .ok_or_else(|| ImpactError::Contract(format!("{flag} requires one value")))?;
            match flag {
                "--artifact-root" => {
                    set_once(&mut artifact_root, PathBuf::from(value), "--artifact-root")?
                }
                "--repo" => set_once(&mut repo, value.clone(), "--repo")?,
                "--repo-path" => set_once(&mut repo_path, PathBuf::from(value), "--repo-path")?,
                "--changed" => changed.push(normalize_relative(value)?),
                _ => unreachable!(),
            }
            index += 2;
        }
        let artifact_root = artifact_root
            .ok_or_else(|| ImpactError::Contract("--artifact-root is required".into()))?;
        let repo_path =
            repo_path.ok_or_else(|| ImpactError::Contract("--repo-path is required".into()))?;
        if !artifact_root.is_dir() || !repo_path.is_dir() {
            return Err(ImpactError::Contract(
                "artifact root and repository path must be existing directories".into(),
            ));
        }
        changed.sort();
        changed.dedup();
        if changed.is_empty() {
            return Err(ImpactError::Contract(
                "at least one --changed path is required".into(),
            ));
        }
        Ok(Self {
            artifact_root,
            repo: repo.ok_or_else(|| ImpactError::Contract("--repo is required".into()))?,
            repo_path,
            changed,
        })
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), ImpactError> {
    if slot.replace(value).is_some() {
        Err(ImpactError::Contract(format!("duplicate {flag}")))
    } else {
        Ok(())
    }
}

fn execute(cli: Cli) -> Result<Value, ImpactError> {
    let evidence = committed_evidence::load(&cli.artifact_root, &cli.repo).map_err(map_evidence)?;
    let run_outcome = evidence.entry["outcome"]
        .as_str()
        .expect("A08 entry outcome");
    if run_outcome != "completed" {
        return Err(ImpactError::Contract(format!(
            "change impact requires a completed authoritative run; latest committed run outcome is {run_outcome}"
        )));
    }
    let freshness = evidence
        .freshness(Some(&cli.repo_path))
        .map_err(map_evidence)?;
    if freshness["status"] != "current" {
        return Err(ImpactError::Contract(format!(
            "change impact requires the committed snapshot to be current; recorded={} current={}",
            freshness["recordedIdentity"].as_str().unwrap_or("unknown"),
            freshness["currentIdentity"].as_str().unwrap_or("unknown")
        )));
    }
    let (files_ref, files_artifact) = evidence
        .artifact("code_evidence.files")
        .ok_or_else(|| ImpactError::Contract("committed run lacks code_evidence.files".into()))?;
    let (imports_ref, imports_artifact) = evidence
        .artifact("code_evidence.imports")
        .ok_or_else(|| ImpactError::Contract("committed run lacks code_evidence.imports".into()))?;
    let files_json: Value = serde_json::from_slice(files_artifact.bytes())
        .map_err(|_| ImpactError::Contract("code_evidence.files is invalid JSON".into()))?;
    let imports_json: Value = serde_json::from_slice(imports_artifact.bytes())
        .map_err(|_| ImpactError::Contract("code_evidence.imports is invalid JSON".into()))?;
    let files = files_json["files"]
        .as_array()
        .expect("registered native files artifact");
    let file_paths = files
        .iter()
        .map(|file| file["path"].as_str().unwrap().to_string())
        .collect::<BTreeSet<_>>();
    let imports = imports_json["imports"]
        .as_array()
        .expect("registered native imports artifact");
    let (reverse, resolved_edges, unresolved_edges) = reverse_import_graph(imports, &file_paths);
    let impacted = impacted_files(&cli.changed, &file_paths, &reverse);
    let test_files = select_tests(&impacted, &cli.changed, &file_paths);
    let commands = test_commands(&test_files);
    let changed = cli
        .changed
        .iter()
        .map(|path| json!({"path":path,"inInventory":file_paths.contains(path)}))
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
    Ok(json!({
        "schema":"code-intel-change-impact.v1",
        "repo":cli.repo,
        "run":evidence.entry["run"],
        "runIdentity":evidence.entry["runIdentity"],
        "runOutcome":run_outcome,
        "snapshotIdentity":evidence.snapshot_identity(),
        "freshness":freshness,
        "changed":changed,
        "evidenceRefs":[files_ref,imports_ref],
        "impact":{
            "files":impact_rows,
            "resolvedImportEdges":resolved_edges,
            "unresolvedImportEdges":unresolved_edges,
        },
        "testSelection":{
            "status":if test_files.is_empty() { "none" } else { "candidates" },
            "files":test_files,
            "commands":commands,
            "advisoryOnly":true,
            "rationale":"Select impacted test files reachable through the verified snapshot's reverse import graph; use same-module test co-location only as a fallback.",
        },
        "limitations":[
            "Native import extraction is heuristic and does not prove runtime call paths.",
            "Dynamic imports, generated code, reflection, build-system edges, and external packages may be unresolved.",
            "Test commands are candidates only and are never executed by this command."
        ]
    }))
}

#[derive(Clone)]
struct ReverseEdge {
    importer: String,
    confidence: &'static str,
}

fn reverse_import_graph(
    imports: &[Value],
    files: &BTreeSet<String>,
) -> (BTreeMap<String, Vec<ReverseEdge>>, usize, usize) {
    let mut reverse: BTreeMap<String, Vec<ReverseEdge>> = BTreeMap::new();
    let mut resolved = 0;
    let mut unresolved = 0;
    for import in imports {
        let importer = import["file"].as_str().unwrap();
        let target = import["target"].as_str().unwrap();
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
    (reverse, resolved, unresolved)
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

struct ImpactReason {
    distance: usize,
    reason: &'static str,
    via: Option<String>,
    confidence: &'static str,
}

fn impacted_files(
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

fn select_tests(
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

fn test_commands(tests: &[String]) -> Vec<String> {
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

fn test_file(path: &str) -> bool {
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

fn normalize_relative(path: &str) -> Result<String, ImpactError> {
    let path = path.replace('\\', "/");
    if path.is_empty()
        || path.starts_with('/')
        || path.contains(':')
        || path
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(ImpactError::Contract(format!(
            "--changed must be a portable repository-relative path: {path}"
        )));
    }
    Ok(path)
}

fn map_evidence(error: EvidenceError) -> ImpactError {
    match error {
        EvidenceError::Contract(message) => ImpactError::Contract(message),
        EvidenceError::HostIo(message) => ImpactError::HostIo(message),
    }
}

enum ImpactError {
    Contract(String),
    HostIo(String),
}
