//! Issue #386: project already-committed, snapshot-bound Sentrux capability
//! artifacts (`sentrux.scan`, `sentrux.health`, `sentrux.check`,
//! `sentrux.gate`) into a versioned Quality Signal + finding artifact for PR
//! Check display and an Orca-consumable lifecycle event.
//!
//! This module is a *consumer*, not a producer:
//! - It does not compute the Quality Signal formula (`sentrux_gate.rs`,
//!   #385's scope) -- it reads whatever total/root-cause/bottleneck values
//!   the engine already emitted into `sentrux.scan`/`sentrux.health`'s
//!   verified `structuredData`.
//! - It does not dispatch any capability (`builtin_provider_evidence.rs`) --
//!   it reads capability artifacts already committed by `run execute` and
//!   verified (sha256-content-addressed) by `committed_evidence::load`.
//! - It never mutates GitHub or Orca state itself; it only emits JSON to
//!   stdout/`--out`. A workflow step (`.github/workflows/pr-gate.yml`) reads
//!   that JSON and performs any GitHub-facing side effect, exactly like the
//!   existing `change risk` sticky-comment job does with `risk.json`.
//!
//! Issue #383 is this module's hard prerequisite: `structuredData` used to
//! silently become `Value::Null` for any real capability output over 8KB
//! (`capability_structured_data` reparsing a bounded 8KB preview). Every
//! field this module reads from `outputs.structuredData` depends on that fix
//! (`sentrux_command.rs`/`sentrux_capability_artifacts.rs`) having landed.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::capability::sha256_hex;
use crate::committed_evidence::{self, CommittedEvidence};
use crate::snapshot;

/// The versioned, snapshot-bound artifact this module produces.
pub(crate) const PROJECTION_SCHEMA: &str = "code-intel-quality-signal-projection.v1";
const PROJECTION_CONTRACT_VERSION: i64 = 1;

/// The small, Orca-consumable lifecycle event nested inside the projection
/// artifact. Kept versioned independently so an Orca-side consumer can pin
/// to just this shape without parsing the (potentially much larger) full
/// findings list.
pub(crate) const ORCA_EVENT_SCHEMA: &str = "code-intel-orca-quality-event.v1";
const ORCA_EVENT_CONTRACT_VERSION: i64 = 1;

const SENTRUX_CAPABILITY_ARTIFACT_SCHEMA: &str = "code-intel-sentrux-capability-artifact.v1";
const SENTRUX_CAPABILITY_ARTIFACT_TYPE: &str = "provider.sentrux.capability-artifact";

const BASELINE_RELATIVE_PATH: &str = ".sentrux/baseline.json";

#[derive(Debug)]
pub(crate) enum ProjectionError {
    Contract(String),
    HostIo(String),
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(message) | Self::HostIo(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ProjectionError {}

/// The four raw metrics the *current* `sentrux_gate.rs` formula actually
/// scores (`coupling_score * 8`, `complex_fn_count * 60`, `god_file_count *
/// 120`, `(max_complexity - 15).max(0) * 10`), plus `cycle_count`, which the
/// engine measures and gates (`max_cycles`/`cycles_increased`) but does not
/// fold into the scalar total. This is a *proxy* for the upstream Quality
/// Signal's five root causes (modularity/acyclicity/depth/equality/
/// redundancy, #385's scope) under this engine's own honest names -- not a
/// claim that they are the same thing. `root_causes_section` switches to
/// consuming #385's `root_causes.<id>.{raw,score}` shape verbatim the moment
/// a payload carries all five upstream ids.
struct LegacyRootCause {
    id: &'static str,
    label: &'static str,
    metric_key: &'static str,
}

const LEGACY_ROOT_CAUSES: [LegacyRootCause; 5] = [
    LegacyRootCause {
        id: "coupling",
        label: "Coupling",
        metric_key: "coupling_score",
    },
    LegacyRootCause {
        id: "complexity",
        label: "Complex functions",
        metric_key: "complex_fn_count",
    },
    LegacyRootCause {
        id: "godFiles",
        label: "God files",
        metric_key: "god_file_count",
    },
    LegacyRootCause {
        id: "maxComplexity",
        label: "Max complexity",
        metric_key: "max_complexity",
    },
    LegacyRootCause {
        id: "cycles",
        label: "Import cycles",
        metric_key: "cycle_count",
    },
];

const UPSTREAM_ROOT_CAUSE_IDS: [&str; 5] = [
    "modularity",
    "acyclicity",
    "depth",
    "equality",
    "redundancy",
];

pub(crate) struct OrcaCorrelation {
    pub(crate) run_id: Option<String>,
    pub(crate) task_id: Option<String>,
    pub(crate) dispatch_id: Option<String>,
    pub(crate) pr_number: Option<String>,
}

pub(crate) struct ProjectionRequest<'a> {
    pub(crate) evidence: &'a CommittedEvidence,
    pub(crate) repo_path: &'a Path,
    pub(crate) commit: &'a str,
    pub(crate) base_ref: Option<&'a str>,
    pub(crate) correlation: OrcaCorrelation,
}

/// Build the versioned projection artifact.
///
/// Fails deterministically (never silently degrades) when:
/// - `request.commit` does not match the checked-out repository's actual
///   `HEAD` (wrong commit bound to this projection), or
/// - the committed snapshot the capability artifacts were verified against
///   no longer matches the checked-out working tree (`CommittedEvidence`'s
///   own freshness check) -- either case is exactly "snapshot mismatch or
///   tampering" from #386's acceptance criteria.
///
/// Everything else (a missing `sentrux.scan`/`sentrux.health` artifact, an
/// unreadable `.sentrux/baseline.json`, a not-yet-upstream root-cause shape)
/// degrades the `completeness` field and records an honest diagnostic
/// instead of failing the whole command -- a PR Check must still render
/// *something* actionable when only part of the evidence is present.
pub(crate) fn build(request: &ProjectionRequest<'_>) -> Result<Value, ProjectionError> {
    let (repo_identity, head) = snapshot::git_repository_identity(request.repo_path)
        .map_err(ProjectionError::HostIo)?
        .ok_or_else(|| {
            ProjectionError::Contract(format!(
                "{} is not a Git repository; cannot bind commit identity",
                request.repo_path.display()
            ))
        })?;
    if head != request.commit {
        return Err(ProjectionError::Contract(format!(
            "--commit {} does not match the checked-out repository's HEAD {head}; refusing to bind a projection to the wrong commit",
            request.commit
        )));
    }

    let freshness = request
        .evidence
        .freshness(Some(request.repo_path))
        .map_err(|error| {
            let (crate::committed_evidence::EvidenceError::Contract(message)
            | crate::committed_evidence::EvidenceError::HostIo(message)) = error;
            ProjectionError::Contract(message)
        })?;
    if freshness["status"] != "current" {
        return Err(ProjectionError::Contract(format!(
            "committed snapshot {} no longer matches the checked-out working tree (current identity {}); refusing to project against a stale or mismatched snapshot",
            freshness["recordedIdentity"], freshness["currentIdentity"]
        )));
    }

    let mut diagnostics = Vec::new();
    let payloads = sentrux_capability_payloads(request.evidence);
    let scan = find_capability(&payloads, "sentrux.scan");
    let health = find_capability(&payloads, "sentrux.health");
    let check = find_capability(&payloads, "sentrux.check");
    let gate = find_capability(&payloads, "sentrux.gate");

    if scan.is_none() {
        diagnostics.push(
            "No verified sentrux.scan capability artifact is present in the committed manifest; qualitySignal is unavailable.".to_string(),
        );
    }
    if health.is_none() {
        diagnostics.push(
            "No verified sentrux.health capability artifact is present in the committed manifest; bottleneck is unavailable.".to_string(),
        );
    }

    let scan_structured = scan.map(|(_, payload)| &payload["outputs"]["structuredData"]);
    let health_structured = health.map(|(_, payload)| &payload["outputs"]["structuredData"]);

    let baseline_metrics = read_baseline_metrics(request.repo_path, &mut diagnostics);

    let quality_signal = quality_signal_section(
        scan_structured,
        health_structured,
        baseline_metrics.as_ref(),
        &mut diagnostics,
    );

    let mut findings = Vec::new();
    findings.extend(root_cause_finding(
        &quality_signal,
        health.map(|(reference, _)| reference.clone()),
    ));
    findings.extend(violation_findings(check, "sentrux.check"));
    findings.extend(violation_findings(gate, "sentrux.gate"));

    let completeness = if scan.is_none() {
        "unavailable"
    } else if health.is_none() || baseline_metrics.is_none() {
        "degraded"
    } else {
        "complete"
    };

    let finding_counts = finding_counts(&findings);
    let orca_event = json!({
        "schema": ORCA_EVENT_SCHEMA,
        "contractVersion": ORCA_EVENT_CONTRACT_VERSION,
        "eventType": "quality_signal_projection",
        "status": completeness,
        "snapshotIdentity": request.evidence.snapshot_identity(),
        "commit": request.commit,
        "baseRef": request.base_ref,
        "repositoryIdentity": repo_identity,
        "summary": {
            "total": quality_signal["total"],
            "bottleneck": quality_signal["bottleneck"],
            "findingCounts": finding_counts,
        },
        "correlate": {
            "runId": request.correlation.run_id,
            "taskId": request.correlation.task_id,
            "dispatchId": request.correlation.dispatch_id,
            "prNumber": request.correlation.pr_number,
        },
    });

    Ok(json!({
        "schema": PROJECTION_SCHEMA,
        "contractVersion": PROJECTION_CONTRACT_VERSION,
        "repo": request.evidence.entry["repo"],
        "run": request.evidence.entry["run"],
        "snapshotIdentity": request.evidence.snapshot_identity(),
        "binding": {
            "commit": request.commit,
            "baseRef": request.base_ref,
            "repositoryIdentity": repo_identity,
            "snapshotIdentity": request.evidence.snapshot_identity(),
        },
        "completeness": completeness,
        "diagnostics": diagnostics,
        "qualitySignal": quality_signal,
        "findings": findings,
        "orcaEvent": orca_event,
    }))
}

fn sentrux_capability_payloads(evidence: &CommittedEvidence) -> Vec<(Value, Value)> {
    evidence
        .refs
        .iter()
        .zip(evidence.verified.iter())
        .filter(|(reference, _)| {
            reference["artifactSchema"] == SENTRUX_CAPABILITY_ARTIFACT_SCHEMA
                && reference["type"] == SENTRUX_CAPABILITY_ARTIFACT_TYPE
        })
        .filter_map(|(reference, verified)| {
            serde_json::from_slice::<Value>(verified.bytes())
                .ok()
                .map(|payload| (reference.clone(), payload))
        })
        .collect()
}

fn find_capability<'a>(
    payloads: &'a [(Value, Value)],
    capability_id: &str,
) -> Option<(&'a Value, &'a Value)> {
    payloads
        .iter()
        .find(|(_, payload)| payload["capabilityId"] == capability_id)
        .map(|(reference, payload)| (reference, payload))
}

fn read_baseline_metrics(repo_path: &Path, diagnostics: &mut Vec<String>) -> Option<Value> {
    let path = repo_path.join(BASELINE_RELATIVE_PATH);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            diagnostics.push(format!(
                "{BASELINE_RELATIVE_PATH} is absent; before/delta values are unavailable."
            ));
            return None;
        }
        Err(error) => {
            diagnostics.push(format!(
                "{BASELINE_RELATIVE_PATH} could not be read ({error}); before/delta values are unavailable."
            ));
            return None;
        }
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(document) if document["metrics"].is_object() => Some(document["metrics"].clone()),
        Ok(_) => {
            diagnostics.push(format!(
                "{BASELINE_RELATIVE_PATH} has no `metrics` object; before/delta values are unavailable."
            ));
            None
        }
        Err(error) => {
            diagnostics.push(format!(
                "{BASELINE_RELATIVE_PATH} is not valid JSON ({error}); before/delta values are unavailable."
            ));
            None
        }
    }
}

fn quality_signal_section(
    scan_structured: Option<&Value>,
    health_structured: Option<&Value>,
    baseline_metrics: Option<&Value>,
    diagnostics: &mut Vec<String>,
) -> Value {
    let current_total = quality_signal_total(
        scan_structured,
        "The verified sentrux.scan payload",
        "current/delta",
        diagnostics,
    );
    let baseline_total = quality_signal_total(
        baseline_metrics,
        BASELINE_RELATIVE_PATH,
        "baseline/delta",
        diagnostics,
    );
    let delta_total = match (current_total, baseline_total) {
        (Some(current), Some(baseline)) => Some(current - baseline),
        _ => None,
    };

    let bottleneck = health_structured
        .and_then(|value| value["bottleneck"].as_str())
        .and_then(normalize_bottleneck_id);

    let (root_causes, formula_version) =
        root_causes_section(scan_structured, baseline_metrics, diagnostics);

    json!({
        "total": {"current": current_total, "baseline": baseline_total, "delta": delta_total},
        "bottleneck": bottleneck,
        "formulaVersion": formula_version,
        "rootCauses": root_causes,
    })
}

fn quality_signal_total(
    source: Option<&Value>,
    source_name: &str,
    unavailable_values: &str,
    diagnostics: &mut Vec<String>,
) -> Option<i64> {
    let source = source?;
    let total = integral_numeric_total(&source["quality_signal"]);
    if total.is_none() {
        diagnostics.push(format!(
            "{source_name} has no usable integral numeric `quality_signal`; {unavailable_values} total values are unavailable."
        ));
    }
    total
}

fn integral_numeric_total(value: &Value) -> Option<i64> {
    let number = value.as_number()?;
    if let Some(integer) = number.as_i64() {
        return Some(integer);
    }
    let float = number.as_f64()?;
    if !float.is_finite()
        || float.fract() != 0.0
        || float < i64::MIN as f64
        || float >= -(i64::MIN as f64)
    {
        return None;
    }
    let integer = float as i64;
    (integer as f64 == float).then_some(integer)
}

fn normalize_bottleneck_id(raw: &str) -> Option<&'static str> {
    match raw {
        "god_files" => Some("godFiles"),
        "complexity" => Some("complexity"),
        "coupling" => Some("coupling"),
        "none" => None,
        _ => None,
    }
}

/// Reads #385's `root_causes.<id>.{raw,score}` shape verbatim when a payload
/// carries all five upstream ids; otherwise projects this engine's own
/// currently-measured proxy metrics under `LEGACY_ROOT_CAUSES`'s honest
/// names, with `score: null` (this module never invents a score by
/// multiplying a raw metric by a weight it does not own -- that is #385's
/// formula).
fn root_causes_section(
    scan_structured: Option<&Value>,
    baseline_metrics: Option<&Value>,
    diagnostics: &mut Vec<String>,
) -> (Value, Value) {
    if let Some(scan_structured) = scan_structured {
        if let Some(upstream) = upstream_root_causes(scan_structured) {
            let formula_version = scan_structured["formula_version"]
                .as_str()
                .map(Value::from)
                .unwrap_or(Value::Null);
            return (Value::Array(upstream), formula_version);
        }
    }
    diagnostics.push(
        "The verified sentrux.scan payload has no upstream `root_causes.<id>` shape yet (#385 pending); projecting this engine's own currently-measured proxy metrics instead.".to_string(),
    );
    let entries = LEGACY_ROOT_CAUSES
        .iter()
        .map(|cause| {
            let current = scan_structured.and_then(|value| value[cause.metric_key].as_f64());
            let baseline = baseline_metrics.and_then(|value| value[cause.metric_key].as_f64());
            let delta = match (current, baseline) {
                (Some(current), Some(baseline)) => Some(current - baseline),
                _ => None,
            };
            json!({
                "id": cause.id,
                "label": cause.label,
                "raw": {"current": current, "baseline": baseline, "delta": delta},
                "score": Value::Null,
                "scoreStatus": "pending_upstream_formula",
            })
        })
        .collect::<Vec<_>>();
    (Value::Array(entries), json!("legacy_proxy_v0"))
}

fn upstream_root_causes(scan_structured: &Value) -> Option<Vec<Value>> {
    let root_causes = scan_structured.get("root_causes")?.as_object()?;
    let mut entries = Vec::with_capacity(UPSTREAM_ROOT_CAUSE_IDS.len());
    for id in UPSTREAM_ROOT_CAUSE_IDS {
        let entry = root_causes.get(id)?;
        let raw = entry.get("raw")?.as_f64()?;
        let score = entry.get("score")?.as_f64()?;
        entries.push(json!({
            "id": id,
            "label": id,
            "raw": {"current": raw, "baseline": Value::Null, "delta": Value::Null},
            "score": score,
            "scoreStatus": "upstream",
        }));
    }
    Some(entries)
}

fn root_cause_finding(quality_signal: &Value, health_ref: Option<Value>) -> Option<Value> {
    let bottleneck = quality_signal["bottleneck"].as_str()?;
    let fingerprint =
        finding_fingerprint("root_cause_diagnostic", "sentrux.health", bottleneck, &[]);
    Some(json!({
        "fingerprint": fingerprint,
        "kind": "root_cause_diagnostic",
        "capabilityId": "sentrux.health",
        "rule": bottleneck,
        "message": format!("Quality Signal bottleneck: {bottleneck}"),
        "targets": Value::Array(Vec::new()),
        "severityUpstream": Value::Null,
        "severityNormalized": "medium",
        "evidenceRefs": health_ref.map(|r| vec![r]).unwrap_or_default(),
    }))
}

fn violation_findings(capability: Option<(&Value, &Value)>, capability_id: &str) -> Vec<Value> {
    let Some((reference, payload)) = capability else {
        return Vec::new();
    };
    let Some(violations) = payload["outputs"]["command"]["violations"].as_array() else {
        return Vec::new();
    };
    violations
        .iter()
        .map(|violation| {
            let rule = violation["rule"].as_str().unwrap_or("unknown_rule");
            let message = violation["message"].as_str().unwrap_or("");
            let targets = violation["targets"]
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let kind = violation_kind(rule);
            let fingerprint = finding_fingerprint(kind, capability_id, rule, &targets);
            json!({
                "fingerprint": fingerprint,
                "kind": kind,
                "capabilityId": capability_id,
                "rule": rule,
                "message": message,
                "targets": targets,
                "severityUpstream": Value::Null,
                "severityNormalized": violation_severity(rule),
                "evidenceRefs": [reference.clone()],
            })
        })
        .collect()
}

/// `quality_degraded`/`coupling_increased`/`cycles_increased`/
/// `god_files_increased` are baseline-ratchet regressions
/// (`sentrux_gate.rs`'s `run_gate`, comparing against `.sentrux/baseline.json`).
/// Everything else -- `.sentrux/rules.toml` threshold rules
/// (`max_cc`/`no_god_files`/`max_cycles`/`max_coupling`), the two governance
/// violations (`baseline_missing`/`baseline_engine_mismatch`), and any future
/// rule this table does not yet know about -- defaults to `rule_violation`,
/// the safe default: it is never silently dropped from `findings`, and is
/// never mislabelled as a ratchet regression it is not.
fn violation_kind(rule: &str) -> &'static str {
    match rule {
        "quality_degraded" | "coupling_increased" | "cycles_increased" | "god_files_increased" => {
            "baseline_regression"
        }
        _ => "rule_violation",
    }
}

fn violation_severity(rule: &str) -> &'static str {
    match rule {
        "quality_degraded" | "coupling_increased" | "cycles_increased" | "god_files_increased" => {
            "high"
        }
        "baseline_missing" | "baseline_engine_mismatch" => "low",
        _ => "medium",
    }
}

/// Stable across reruns of the *same* underlying issue: built from the
/// finding's identity (kind, rule/metric, capability, sorted targets), never
/// from its message text or any run/snapshot-specific value -- a message
/// that only changed its embedded "12 -> 8" numbers must not mint a new
/// fingerprint, or a consumer diffing two projection runs could never tell
/// "still failing" from "newly failing".
fn finding_fingerprint(kind: &str, capability_id: &str, rule: &str, targets: &[String]) -> String {
    let mut sorted_targets = targets.to_vec();
    sorted_targets.sort();
    let identity = format!("{kind}|{capability_id}|{rule}|{}", sorted_targets.join(","));
    format!("sha256:{}", sha256_hex(identity.as_bytes()))
}

fn finding_counts(findings: &[Value]) -> Value {
    let mut by_kind = std::collections::BTreeMap::<&str, i64>::new();
    let mut by_severity = std::collections::BTreeMap::<&str, i64>::new();
    for finding in findings {
        if let Some(kind) = finding["kind"].as_str() {
            *by_kind.entry(kind).or_insert(0) += 1;
        }
        if let Some(severity) = finding["severityNormalized"].as_str() {
            *by_severity.entry(severity).or_insert(0) += 1;
        }
    }
    json!({
        "total": findings.len(),
        "byKind": by_kind,
        "bySeverity": by_severity,
    })
}

/// Process-facing CLI seam: `code-intel quality-projection build`. Reads
/// already-committed evidence (`committed_evidence::load`), builds the
/// projection, prints it to stdout, and optionally writes it to `--out`. The
/// workflow step invoking this binary owns every GitHub-facing side effect
/// (sticky comment, Check summary) -- this command only ever emits JSON.
pub(crate) fn run_raw(raw: &[String]) -> i32 {
    match parse_cli(raw).and_then(execute_cli) {
        Ok(value) => {
            println!("{}", serde_json::to_string(&value).unwrap());
            0
        }
        Err(ProjectionError::Contract(message)) => {
            eprintln!("{message}");
            65
        }
        Err(ProjectionError::HostIo(message)) => {
            eprintln!("{message}");
            74
        }
    }
}

struct Cli {
    artifact_root: PathBuf,
    repo: String,
    repo_path: PathBuf,
    commit: String,
    base_ref: Option<String>,
    out: Option<PathBuf>,
    orca_run_id: Option<String>,
    orca_task_id: Option<String>,
    orca_dispatch_id: Option<String>,
    pr_number: Option<String>,
}

const USAGE: &str = "usage: quality-projection build --artifact-root <root> --repo <name> --repo-path <checkout> --commit <sha> [--base-ref <ref>] [--out <path>] [--orca-run-id <id>] [--orca-task-id <id>] [--orca-dispatch-id <id>] [--pr <number>]";

fn parse_cli(raw: &[String]) -> Result<Cli, ProjectionError> {
    if raw.first().map(String::as_str) != Some("build") {
        return Err(ProjectionError::Contract(USAGE.into()));
    }
    let mut artifact_root = None;
    let mut repo = None;
    let mut repo_path = None;
    let mut commit = None;
    let mut base_ref = None;
    let mut out = None;
    let mut orca_run_id = None;
    let mut orca_task_id = None;
    let mut orca_dispatch_id = None;
    let mut pr_number = None;
    let mut index = 1;
    while index < raw.len() {
        let flag = raw[index].as_str();
        if !matches!(
            flag,
            "--artifact-root"
                | "--repo"
                | "--repo-path"
                | "--commit"
                | "--base-ref"
                | "--out"
                | "--orca-run-id"
                | "--orca-task-id"
                | "--orca-dispatch-id"
                | "--pr"
        ) {
            return Err(ProjectionError::Contract(format!(
                "unknown quality-projection build argument: {flag}"
            )));
        }
        let value = raw
            .get(index + 1)
            .filter(|value| !value.is_empty() && !value.starts_with("--"))
            .ok_or_else(|| ProjectionError::Contract(format!("{flag} requires one value")))?;
        match flag {
            "--artifact-root" => set_once(&mut artifact_root, PathBuf::from(value), flag)?,
            "--repo" => set_once(&mut repo, value.clone(), flag)?,
            "--repo-path" => set_once(&mut repo_path, PathBuf::from(value), flag)?,
            "--commit" => set_once(&mut commit, value.clone(), flag)?,
            "--base-ref" => set_once(&mut base_ref, value.clone(), flag)?,
            "--out" => set_once(&mut out, PathBuf::from(value), flag)?,
            "--orca-run-id" => set_once(&mut orca_run_id, value.clone(), flag)?,
            "--orca-task-id" => set_once(&mut orca_task_id, value.clone(), flag)?,
            "--orca-dispatch-id" => set_once(&mut orca_dispatch_id, value.clone(), flag)?,
            "--pr" => set_once(&mut pr_number, value.clone(), flag)?,
            _ => unreachable!(),
        }
        index += 2;
    }
    let artifact_root = artifact_root
        .ok_or_else(|| ProjectionError::Contract("--artifact-root is required".into()))?;
    if !artifact_root.is_dir() {
        return Err(ProjectionError::Contract(
            "--artifact-root must be an existing directory".into(),
        ));
    }
    let repo = repo.ok_or_else(|| ProjectionError::Contract("--repo is required".into()))?;
    let repo_path =
        repo_path.ok_or_else(|| ProjectionError::Contract("--repo-path is required".into()))?;
    if !repo_path.is_dir() {
        return Err(ProjectionError::Contract(
            "--repo-path must be an existing directory".into(),
        ));
    }
    let commit = commit.ok_or_else(|| ProjectionError::Contract("--commit is required".into()))?;
    Ok(Cli {
        artifact_root,
        repo,
        repo_path,
        commit,
        base_ref,
        out,
        orca_run_id,
        orca_task_id,
        orca_dispatch_id,
        pr_number,
    })
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), ProjectionError> {
    if slot.replace(value).is_some() {
        Err(ProjectionError::Contract(format!("duplicate {flag}")))
    } else {
        Ok(())
    }
}

fn execute_cli(cli: Cli) -> Result<Value, ProjectionError> {
    let evidence =
        committed_evidence::load(&cli.artifact_root, &cli.repo).map_err(map_evidence_error)?;
    let request = ProjectionRequest {
        evidence: &evidence,
        repo_path: &cli.repo_path,
        commit: &cli.commit,
        base_ref: cli.base_ref.as_deref(),
        correlation: OrcaCorrelation {
            run_id: cli.orca_run_id,
            task_id: cli.orca_task_id,
            dispatch_id: cli.orca_dispatch_id,
            pr_number: cli.pr_number,
        },
    };
    let value = build(&request)?;
    if let Some(path) = &cli.out {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                ProjectionError::HostIo(format!("create --out directory: {error}"))
            })?;
        }
        fs::write(
            path,
            serde_json::to_vec_pretty(&value).expect("projection artifact serializes"),
        )
        .map_err(|error| ProjectionError::HostIo(format!("write --out: {error}")))?;
    }
    Ok(value)
}

fn map_evidence_error(error: committed_evidence::EvidenceError) -> ProjectionError {
    match error {
        committed_evidence::EvidenceError::Contract(message) => ProjectionError::Contract(message),
        committed_evidence::EvidenceError::HostIo(message) => ProjectionError::HostIo(message),
    }
}

#[cfg(test)]
#[path = "sentrux_quality_projection_tests.rs"]
mod tests;
