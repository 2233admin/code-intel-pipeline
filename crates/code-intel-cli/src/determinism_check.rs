//! R1.1: a measurement-determinism gate over `diagnosis.hospital`'s reachable
//! structured evidence chain (PRD 2026-08-06 §M1, pinned decision #1: three
//! consecutive `run execute` runs against the same commit, compared after
//! stripping timestamp/run-identity fields; any pairwise mismatch fails
//! closed).
//!
//! Scope is deliberately narrower than "everything `run execute` writes": it
//! tracks only the node output directories `dag_run.rs` wires as
//! `diagnosis.hospital` inputs or as `diagnosis.hospital` itself
//! (`evidence.graph`, `evidence.sentrux`, `diagnosis.hospital`), plus
//! `evidence.native-code` because the PRD names it in the same breath (the
//! native-code enrichment evidence `hospital_diagnosis.rs::consume_admission`
//! knows how to read) even though the production DAG does not currently wire
//! an edge from `evidence.native-code` into `diagnosis.hospital` — see the
//! R1.1 handoff note for that finding. It does not diff the full
//! `run-manifest.json`, which carries a legitimate per-publication identity
//! even when every node underneath it reproduced exactly.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::authoritative_run::{self, RunRequest};
use crate::execution_policy::{ExecutionPolicy, RunProfile};

/// The `diagnosis.hospital`-reachable node output directories, named exactly
/// as `dag_run.rs`'s `NodeSpec::new` ids (they double as the `--out`
/// subdirectory a node's artifacts are staged under).
const TRACKED_NODES: [&str; 4] = [
    "evidence.graph",
    "evidence.sentrux",
    "evidence.native-code",
    "diagnosis.hospital",
];

/// JSON object keys that are allowed to vary run to run without indicating
/// non-determinism in the underlying evidence — wall-clock stamps and
/// per-run/per-observation identifiers — stripped before comparison. Named
/// generically per the PRD's own wording ("generatedAt"/"runIdentity"/
/// timestamp-class fields); see the module doc and the R1.1 handoff note for
/// the concrete fields this was derived from
/// (`builtin_provider_evidence.rs`'s `observedAt`/`collectedAt`,
/// `capability.rs::base_result`'s `provenance.attemptId`/`generatedAt`,
/// `evidence.provenance.collectionId` — an
/// `<implementation-id>-<git-sha>-<unix-timestamp>` string a live self-scan
/// surfaced as a real first-run divergence during R1.1's own repro before
/// this key was added; see the handoff note).
const VOLATILE_KEYS: [&str; 10] = [
    "generatedAt",
    "observedAt",
    "collectedAt",
    "startedAt",
    "completedAt",
    "durationMs",
    "attemptId",
    "runIdentity",
    "admissionIdentity",
    "collectionId",
];

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let cli = match Cli::parse(raw) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}");
            return 64;
        }
    };
    match check(&cli) {
        Ok(report) => {
            let consistent = report["consistent"] == true;
            println!(
                "{}",
                serde_json::to_string(&report).expect("determinism report serializes")
            );
            if consistent {
                0
            } else {
                20
            }
        }
        Err((code, message)) => {
            eprintln!("{message}");
            code
        }
    }
}

#[derive(Debug)]
struct Cli {
    repo: PathBuf,
    out: PathBuf,
    runs: usize,
    manifest: Option<PathBuf>,
    max_concurrency: usize,
    profile: RunProfile,
    doctor_tool_path_prefix: Option<PathBuf>,
    doctor_require_repowise: Option<bool>,
    doctor_require_understand: Option<bool>,
}

impl Cli {
    fn parse(raw: &[String]) -> Result<Self, String> {
        if raw.first().map(String::as_str) != Some("check") {
            return Err(usage());
        }
        let mut repo = None;
        let mut out = None;
        let mut runs = 3usize;
        let mut manifest = None;
        let mut max_concurrency = 2usize;
        let mut profile = RunProfile::Default;
        let mut doctor_tool_path_prefix = None;
        let mut doctor_require_repowise = None;
        let mut doctor_require_understand = None;
        let mut index = 1;
        while index < raw.len() {
            let flag = raw[index].as_str();
            if !matches!(
                flag,
                "--repo"
                    | "--out"
                    | "--runs"
                    | "--manifest"
                    | "--max-concurrency"
                    | "--profile"
                    | "--doctor-tool-path-prefix"
                    | "--doctor-require-repowise"
                    | "--doctor-require-understand"
            ) {
                return Err(format!("unknown determinism check argument: {flag}"));
            }
            let value = raw
                .get(index + 1)
                .filter(|value| !value.is_empty() && !value.starts_with("--"))
                .ok_or_else(|| format!("{flag} requires one value"))?;
            match flag {
                "--repo" if repo.replace(PathBuf::from(value)).is_some() => {
                    return Err("duplicate --repo".into())
                }
                "--repo" => {}
                "--out" if out.replace(PathBuf::from(value)).is_some() => {
                    return Err("duplicate --out".into())
                }
                "--out" => {}
                "--runs" => {
                    runs = value
                        .parse::<usize>()
                        .map_err(|_| "--runs must be an integer".to_string())?;
                    if runs < 2 {
                        return Err("--runs must be at least 2".into());
                    }
                }
                "--manifest" if manifest.replace(PathBuf::from(value)).is_some() => {
                    return Err("duplicate --manifest".into())
                }
                "--manifest" => {}
                "--max-concurrency" => {
                    max_concurrency = value
                        .parse::<usize>()
                        .map_err(|_| "--max-concurrency must be an integer".to_string())?;
                }
                "--profile" => {
                    profile = RunProfile::parse(value)?;
                }
                "--doctor-tool-path-prefix"
                    if doctor_tool_path_prefix
                        .replace(PathBuf::from(value))
                        .is_some() =>
                {
                    return Err("duplicate --doctor-tool-path-prefix".into())
                }
                "--doctor-tool-path-prefix" => {}
                "--doctor-require-repowise" => {
                    doctor_require_repowise = Some(parse_bool_flag(flag, value)?);
                }
                "--doctor-require-understand" => {
                    doctor_require_understand = Some(parse_bool_flag(flag, value)?);
                }
                _ => unreachable!(),
            }
            index += 2;
        }
        let repo = repo.ok_or("--repo is required")?;
        if !repo.is_dir() {
            return Err(format!(
                "repository path is not a directory: {}",
                repo.display()
            ));
        }
        let out = out.ok_or("--out is required")?;
        if let Some(prefix) = &doctor_tool_path_prefix {
            if !prefix.is_dir() {
                return Err(format!(
                    "--doctor-tool-path-prefix is not a directory: {}",
                    prefix.display()
                ));
            }
        }
        Ok(Self {
            repo,
            out,
            runs,
            manifest,
            max_concurrency,
            profile,
            doctor_tool_path_prefix,
            doctor_require_repowise,
            doctor_require_understand,
        })
    }
}

fn parse_bool_flag(flag: &str, value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("{flag} must be true or false")),
    }
}

fn usage() -> String {
    "usage: determinism check --repo <repo-root> --out <staging-parent-directory> \
     [--runs <n>=3] [--profile default|strict|offline] [--manifest <integrations.json>] \
     [--max-concurrency <n>] [--doctor-tool-path-prefix <directory>] \
     [--doctor-require-repowise <true|false>] [--doctor-require-understand <true|false>]"
        .into()
}

struct RunSnapshot {
    index: usize,
    outcome: String,
    exit_code: i32,
    documents: BTreeMap<String, Comparable>,
}

enum Comparable {
    Json(Value),
    Bytes(Vec<u8>),
}

fn check(cli: &Cli) -> Result<Value, (i32, String)> {
    fs::create_dir_all(&cli.out)
        .map_err(|error| (74, format!("create determinism staging root: {error}")))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| (74, format!("read clock: {error}")))?
        .as_nanos();
    let policy = ExecutionPolicy::for_profile(cli.profile).with_doctor_overrides(
        cli.doctor_require_repowise,
        cli.doctor_require_understand,
        cli.doctor_tool_path_prefix.clone(),
    );
    let mut snapshots = Vec::with_capacity(cli.runs);
    for index in 1..=cli.runs {
        let staging = cli.out.join(format!("run-{index}-{nonce}"));
        let authority = cli.out.join(format!("authority-{index}-{nonce}"));
        fs::create_dir_all(&authority).map_err(|error| {
            (
                74,
                format!("create authority root for run {index}: {error}"),
            )
        })?;
        let mut request = RunRequest::new(
            cli.repo.clone(),
            staging.clone(),
            authority,
            format!("determinism-{nonce}-{index}"),
            policy.clone(),
        )
        .with_max_concurrency(cli.max_concurrency);
        if let Some(manifest) = &cli.manifest {
            request = request.with_manifest(Some(manifest.clone()));
        }
        let result = authoritative_run::execute(request).map_err(|error| {
            (
                65,
                format!("run {index} of {}: {}", cli.runs, error.message),
            )
        })?;
        let documents = collect_documents(&staging)?;
        snapshots.push(RunSnapshot {
            index,
            outcome: result.outcome().to_string(),
            exit_code: result.exit_code(),
            documents,
        });
    }
    let divergence = first_divergence(&snapshots);
    Ok(json!({
        "schema": "code-intel-determinism-report.v1",
        "repo": cli.repo,
        "runs": cli.runs,
        "trackedNodes": TRACKED_NODES,
        "runOutcomes": snapshots.iter().map(|snapshot| json!({
            "run": snapshot.index,
            "outcome": snapshot.outcome,
            "exitCode": snapshot.exit_code,
            "documentsCompared": snapshot.documents.len(),
        })).collect::<Vec<_>>(),
        "consistent": divergence.is_none(),
        "firstDivergence": divergence,
    }))
}

fn collect_documents(staging: &Path) -> Result<BTreeMap<String, Comparable>, (i32, String)> {
    let mut documents = BTreeMap::new();
    for node in TRACKED_NODES {
        let node_dir = staging.join(node);
        if !node_dir.is_dir() {
            continue;
        }
        walk(&node_dir, &node_dir, node, &mut documents)?;
    }
    Ok(documents)
}

fn walk(
    root: &Path,
    dir: &Path,
    node: &str,
    out: &mut BTreeMap<String, Comparable>,
) -> Result<(), (i32, String)> {
    let mut entries = fs::read_dir(dir)
        .map_err(|error| (74, format!("read {}: {error}", dir.display())))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| (74, format!("read {}: {error}", dir.display())))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, node, out)?;
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .expect("walked path is under its own root")
            .to_string_lossy()
            .replace('\\', "/");
        let key = format!("{node}/{relative}");
        let bytes =
            fs::read(&path).map_err(|error| (74, format!("read {}: {error}", path.display())))?;
        let comparable = if path.extension().and_then(std::ffi::OsStr::to_str) == Some("json") {
            let mut value: Value = serde_json::from_slice(&bytes)
                .map_err(|error| (74, format!("parse {}: {error}", path.display())))?;
            strip_volatile(&mut value);
            Comparable::Json(value)
        } else {
            Comparable::Bytes(bytes)
        };
        out.insert(key, comparable);
    }
    Ok(())
}

fn strip_volatile(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for key in VOLATILE_KEYS {
                map.remove(key);
            }
            for child in map.values_mut() {
                strip_volatile(child);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                strip_volatile(item);
            }
        }
        _ => {}
    }
}

/// The first pairwise mismatch across every `(run, run)` pair, in a stable
/// scan order — not just adjacent runs, so a run 1/run 3 divergence that
/// happens to agree with run 2 on both sides is still caught (PRD's `C(3,2)`
/// wording).
fn first_divergence(snapshots: &[RunSnapshot]) -> Option<Value> {
    for left in 0..snapshots.len() {
        for right in (left + 1)..snapshots.len() {
            let a = &snapshots[left];
            let b = &snapshots[right];
            if a.outcome != b.outcome || a.exit_code != b.exit_code {
                return Some(json!({
                    "kind": "run_outcome",
                    "runA": a.index,
                    "runB": b.index,
                    "outcomeA": a.outcome,
                    "outcomeB": b.outcome,
                    "exitCodeA": a.exit_code,
                    "exitCodeB": b.exit_code,
                }));
            }
            let mut keys = a
                .documents
                .keys()
                .chain(b.documents.keys())
                .collect::<Vec<_>>();
            keys.sort();
            keys.dedup();
            for key in keys {
                match (a.documents.get(key), b.documents.get(key)) {
                    (Some(left_doc), Some(right_doc)) => {
                        if let Some(detail) = compare(left_doc, right_doc) {
                            return Some(json!({
                                "kind": "document_diff",
                                "runA": a.index,
                                "runB": b.index,
                                "document": key,
                                "detail": detail,
                            }));
                        }
                    }
                    (None, Some(_)) => {
                        return Some(json!({
                            "kind": "document_missing",
                            "runA": a.index,
                            "runB": b.index,
                            "document": key,
                            "presentIn": b.index,
                        }))
                    }
                    (Some(_), None) => {
                        return Some(json!({
                            "kind": "document_missing",
                            "runA": a.index,
                            "runB": b.index,
                            "document": key,
                            "presentIn": a.index,
                        }))
                    }
                    (None, None) => unreachable!("key came from the union of both maps"),
                }
            }
        }
    }
    None
}

fn compare(a: &Comparable, b: &Comparable) -> Option<Value> {
    match (a, b) {
        (Comparable::Json(a), Comparable::Json(b)) => json_diff(a, b, ""),
        (Comparable::Bytes(a), Comparable::Bytes(b)) => (a != b).then(
            || json!({"path": "<raw-bytes>", "note": "byte content differs and is not JSON"}),
        ),
        _ => {
            Some(json!({"path": "<top-level>", "note": "one run produced JSON, the other did not"}))
        }
    }
}

/// The first differing leaf, as a slash-joined path from the document root —
/// deliberately not a full diff: R1.1's acceptance is "print the first
/// divergence point", not an exhaustive report.
fn json_diff(a: &Value, b: &Value, path: &str) -> Option<Value> {
    if a == b {
        return None;
    }
    match (a, b) {
        (Value::Object(left), Value::Object(right)) => {
            let mut keys = left.keys().chain(right.keys()).collect::<Vec<_>>();
            keys.sort();
            keys.dedup();
            for key in keys {
                let child_path = format!("{path}/{key}");
                match (left.get(key), right.get(key)) {
                    (Some(l), Some(r)) => {
                        if let Some(diff) = json_diff(l, r, &child_path) {
                            return Some(diff);
                        }
                    }
                    (None, Some(r)) => {
                        return Some(json!({"path": child_path, "from": Value::Null, "to": r}))
                    }
                    (Some(l), None) => {
                        return Some(json!({"path": child_path, "from": l, "to": Value::Null}))
                    }
                    (None, None) => unreachable!("key came from the union of both objects"),
                }
            }
            None
        }
        (Value::Array(left), Value::Array(right)) => {
            for (index, (l, r)) in left.iter().zip(right.iter()).enumerate() {
                let child_path = format!("{path}[{index}]");
                if let Some(diff) = json_diff(l, r, &child_path) {
                    return Some(diff);
                }
            }
            if left.len() != right.len() {
                return Some(json!({
                    "path": format!("{path}.length"),
                    "from": left.len(),
                    "to": right.len(),
                }));
            }
            None
        }
        _ => Some(json!({"path": path, "from": a, "to": b})),
    }
}

#[cfg(test)]
#[path = "determinism_check_tests.rs"]
mod tests;
