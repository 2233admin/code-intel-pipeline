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
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn cli_parse_rejects_a_missing_subcommand() {
        assert!(Cli::parse(&args(&["--repo", "."])).is_err());
    }

    #[test]
    fn cli_parse_requires_repo_and_out() {
        assert!(Cli::parse(&args(&["check"])).is_err());
        assert!(Cli::parse(&args(&["check", "--repo", "."])).is_err());
    }

    #[test]
    fn cli_parse_rejects_fewer_than_two_runs() {
        let error = Cli::parse(&args(&[
            "check", "--repo", ".", "--out", ".", "--runs", "1",
        ]))
        .unwrap_err();
        assert!(error.contains("--runs must be at least 2"));
    }

    #[test]
    fn cli_parse_rejects_an_unknown_flag() {
        assert!(Cli::parse(&args(&["check", "--repo", ".", "--bogus", "x"])).is_err());
    }

    #[test]
    fn cli_parse_defaults_runs_to_three() {
        let cli = Cli::parse(&args(&["check", "--repo", ".", "--out", "."])).unwrap();
        assert_eq!(cli.runs, 3);
        assert_eq!(cli.max_concurrency, 2);
        assert_eq!(cli.profile, RunProfile::Default);
    }

    #[test]
    fn strip_volatile_removes_every_listed_key_at_any_depth() {
        let mut value = json!({
            "generatedAt": "2026-08-06T00:00:00Z",
            "evidence": {
                "observedAt": 1700000000,
                "provenance": {"collectionId": "graph-internal-abc-1700000000"},
                "rules": [
                    {"admissionIdentity": "sha", "kind": "boundary_dependency"}
                ]
            },
            "kept": "value"
        });
        strip_volatile(&mut value);
        assert_eq!(
            value,
            json!({
                "evidence": {
                    "provenance": {},
                    "rules": [{"kind": "boundary_dependency"}]
                },
                "kept": "value"
            })
        );
    }

    #[test]
    fn json_diff_finds_nothing_for_equal_documents() {
        let a = json!({"a": 1, "b": [1, 2, {"c": "x"}]});
        let b = a.clone();
        assert!(json_diff(&a, &b, "").is_none());
    }

    #[test]
    fn json_diff_reports_the_first_differing_leaf_path() {
        let a = json!({"triage": {"status": "green", "primary_diagnosis": "clean snapshot"}});
        let b = json!({"triage": {"status": "red", "primary_diagnosis": "clean snapshot"}});
        let diff = json_diff(&a, &b, "").expect("documents differ");
        assert_eq!(diff["path"], "/triage/status");
        assert_eq!(diff["from"], "green");
        assert_eq!(diff["to"], "red");
    }

    #[test]
    fn json_diff_reports_array_length_mismatch() {
        let a = json!({"targets": ["a.rs"]});
        let b = json!({"targets": ["a.rs", "b.rs"]});
        let diff = json_diff(&a, &b, "").expect("documents differ");
        assert_eq!(diff["path"], "/targets.length");
    }

    fn snapshot(
        index: usize,
        outcome: &str,
        exit_code: i32,
        documents: &[(&str, Value)],
    ) -> RunSnapshot {
        RunSnapshot {
            index,
            outcome: outcome.to_string(),
            exit_code,
            documents: documents
                .iter()
                .map(|(key, value)| (key.to_string(), Comparable::Json(value.clone())))
                .collect(),
        }
    }

    #[test]
    fn first_divergence_is_none_when_every_run_agrees() {
        let doc = json!({"triage": {"status": "green"}});
        let runs = vec![
            snapshot(
                1,
                "completed",
                0,
                &[("diagnosis.hospital/hospital-report.json", doc.clone())],
            ),
            snapshot(
                2,
                "completed",
                0,
                &[("diagnosis.hospital/hospital-report.json", doc.clone())],
            ),
            snapshot(
                3,
                "completed",
                0,
                &[("diagnosis.hospital/hospital-report.json", doc)],
            ),
        ];
        assert!(first_divergence(&runs).is_none());
    }

    #[test]
    fn first_divergence_catches_a_run_outcome_mismatch_before_diffing_documents() {
        let doc = json!({"triage": {"status": "green"}});
        let runs = vec![
            snapshot(
                1,
                "completed",
                0,
                &[("diagnosis.hospital/hospital-report.json", doc.clone())],
            ),
            snapshot(
                2,
                "domain_failed",
                10,
                &[("diagnosis.hospital/hospital-report.json", doc)],
            ),
        ];
        let divergence = first_divergence(&runs).expect("outcomes differ");
        assert_eq!(divergence["kind"], "run_outcome");
    }

    #[test]
    fn first_divergence_catches_a_document_present_in_only_one_run() {
        let runs = vec![
            snapshot(
                1,
                "completed",
                0,
                &[("evidence.graph/graph-admission.json", json!({}))],
            ),
            snapshot(2, "completed", 0, &[]),
        ];
        let divergence = first_divergence(&runs).expect("document missing in run 2");
        assert_eq!(divergence["kind"], "document_missing");
    }

    #[test]
    fn first_divergence_checks_every_pair_not_just_adjacent_runs() {
        // Run 2 agrees with both neighbours pairwise-adjacently, but run 1
        // and run 3 disagree with each other -- only checking (1,2) and (2,3)
        // would miss it. PRD's `C(3,2)` wording is exactly this: every pair.
        let runs = vec![
            snapshot(
                1,
                "completed",
                0,
                &[("diagnosis.hospital/hospital-report.json", json!({"v": 1}))],
            ),
            snapshot(
                2,
                "completed",
                0,
                &[("diagnosis.hospital/hospital-report.json", json!({"v": 1}))],
            ),
            snapshot(
                3,
                "completed",
                0,
                &[("diagnosis.hospital/hospital-report.json", json!({"v": 2}))],
            ),
        ];
        let divergence = first_divergence(&runs).expect("run 1 and run 3 disagree");
        assert_eq!(divergence["runA"], 1);
        assert_eq!(divergence["runB"], 3);
    }
}
