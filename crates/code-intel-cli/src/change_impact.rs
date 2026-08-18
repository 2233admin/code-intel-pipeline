use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::committed_evidence::{self, CommittedEvidence, EvidenceError};
use crate::impact_graph::{impacted_files, reverse_import_graph, select_tests, test_commands};

const SENTRUX_CAPABILITY_ARTIFACT_SCHEMA: &str = "code-intel-sentrux-capability-artifact.v1";
const SENTRUX_CAPABILITY_ARTIFACT_TYPE: &str = "provider.sentrux.capability-artifact";

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    // Leaf adapter only — controllers wrap the execute_* paths with typed
    // authority receipts and must not be imported here (import cycle).
    let result = ChangeImpactInvocation::parse(raw).and_then(|invocation| match invocation {
        ChangeImpactInvocation::Committed(request) => {
            let evidence = committed_evidence::load(&request.artifact_root, &request.repo)
                .map_err(map_evidence)?;
            execute_committed(request, &evidence).map(|result| result.into_value())
        }
        ChangeImpactInvocation::StaleAdvisory(request) => {
            let evidence = committed_evidence::load(&request.artifact_root, &request.repo)
                .map_err(map_evidence)?;
            execute_stale_advisory(request, &evidence).map(|result| result.into_value())
        }
    });
    match result {
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

#[derive(Clone, Copy, PartialEq)]
enum Staleness {
    Current,
    Advisory,
}

pub(crate) enum ChangeImpactInvocation {
    Committed(ChangeImpactRequest),
    StaleAdvisory(ChangeImpactRequest),
}

pub(crate) struct ChangeImpactRequest {
    pub(crate) artifact_root: PathBuf,
    pub(crate) repo: String,
    pub(crate) repo_path: PathBuf,
    changed: Vec<String>,
}

impl ChangeImpactRequest {
    /// Build a request from already-typed values instead of argv.
    ///
    /// `changed` goes through the same `normalize_relative` guard the
    /// `--changed` flag uses, so a path arriving as a JSON string over the MCP
    /// surface cannot escape the repository by a route the flag parser closes.
    /// Reusing the guard is the point; re-stating it here would be the bug.
    pub(crate) fn new(
        artifact_root: PathBuf,
        repo: String,
        repo_path: PathBuf,
        changed: Vec<String>,
    ) -> Result<Self, ImpactError> {
        let mut changed = changed
            .iter()
            .map(|path| normalize_relative(path))
            .collect::<Result<Vec<_>, _>>()?;
        changed.sort();
        changed.dedup();
        if changed.is_empty() {
            return Err(ImpactError::Contract(
                "at least one changed path is required".into(),
            ));
        }
        Ok(Self {
            artifact_root,
            repo,
            repo_path,
            changed,
        })
    }
}

impl ChangeImpactInvocation {
    pub(crate) fn parse(raw: &[String]) -> Result<Self, ImpactError> {
        if raw.first().map(String::as_str) != Some("impact") {
            return Err(ImpactError::Contract("usage: change impact --artifact-root <root> --repo <name> --repo-path <checkout> --changed <relative-path> [--changed <relative-path>]... [--staleness current|advisory]".into()));
        }
        let mut artifact_root = None;
        let mut repo = None;
        let mut repo_path = None;
        let mut changed = Vec::new();
        let mut staleness = None;
        let mut index = 1;
        while index < raw.len() {
            let flag = raw[index].as_str();
            if !matches!(
                flag,
                "--artifact-root" | "--repo" | "--repo-path" | "--changed" | "--staleness"
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
                "--staleness" => {
                    let mode = match value.as_str() {
                        "current" => Staleness::Current,
                        "advisory" => Staleness::Advisory,
                        other => {
                            return Err(ImpactError::Contract(format!(
                                "--staleness must be current or advisory: {other}"
                            )))
                        }
                    };
                    set_once(&mut staleness, mode, "--staleness")?
                }
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
        let request = ChangeImpactRequest {
            artifact_root,
            repo: repo.ok_or_else(|| ImpactError::Contract("--repo is required".into()))?,
            repo_path,
            changed,
        };
        Ok(match staleness.unwrap_or(Staleness::Current) {
            Staleness::Current => Self::Committed(request),
            Staleness::Advisory => Self::StaleAdvisory(request),
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

pub(crate) struct ChangeImpactResult {
    value: Value,
}

impl ChangeImpactResult {
    pub(crate) fn value(&self) -> &Value {
        &self.value
    }

    pub(crate) fn into_value(self) -> Value {
        self.value
    }
}

pub(crate) fn execute_committed(
    cli: ChangeImpactRequest,
    evidence: &CommittedEvidence,
) -> Result<ChangeImpactResult, ImpactError> {
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
    build_result(cli, evidence, freshness, false)
}

pub(crate) fn execute_stale_advisory(
    cli: ChangeImpactRequest,
    evidence: &CommittedEvidence,
) -> Result<ChangeImpactResult, ImpactError> {
    let mut freshness = evidence
        .freshness(Some(&cli.repo_path))
        .map_err(map_evidence)?;
    let stale = freshness["status"] != "current";
    if stale {
        freshness["status"] = json!("stale-advisory");
    }
    build_result(cli, evidence, freshness, stale)
}

fn build_result(
    cli: ChangeImpactRequest,
    evidence: &CommittedEvidence,
    freshness: Value,
    stale: bool,
) -> Result<ChangeImpactResult, ImpactError> {
    let run_outcome = evidence.entry["outcome"]
        .as_str()
        .expect("A08 entry outcome");
    let stale_identities = (freshness["status"] == "stale-advisory").then(|| {
        (
            freshness["recordedIdentity"].clone(),
            freshness["currentIdentity"].clone(),
        )
    });
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
        .map(|file| {
            file["path"].as_str().map(str::to_string).ok_or_else(|| {
                ImpactError::Contract("code_evidence.files entries must carry a string path".into())
            })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let imports = imports_json["imports"]
        .as_array()
        .expect("registered native imports artifact");
    let (reverse, resolved_edges, unresolved_edges) =
        reverse_import_graph(imports, &file_paths).map_err(ImpactError::Contract)?;
    let impacted = impacted_files(&cli.changed, &file_paths, &reverse);
    let test_files = select_tests(&impacted, &cli.changed, &file_paths);
    let co_location_fallback =
        !test_files.is_empty() && !test_files.iter().any(|path| impacted.contains_key(path));
    let (commands, command_limitations) = test_commands(
        &cli.repo_path,
        &cli.changed,
        &test_files,
        co_location_fallback,
    );
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
    let (sentrux_evidence_refs, sentrux_evidence) = sentrux_evidence(evidence, stale);
    let sentrux_signals = sentrux_test_selection_signals(evidence, stale);
    let mut result = json!({
        "schema":"code-intel-change-impact.v1",
        "repo":cli.repo,
        "run":evidence.entry["run"],
        "runIdentity":evidence.entry["runIdentity"],
        "runOutcome":run_outcome,
        "snapshotIdentity":evidence.snapshot_identity(),
        "freshness":freshness,
        "changed":changed,
        "evidenceRefs":[files_ref,imports_ref],
        "sentruxEvidenceRefs":sentrux_evidence_refs,
        "sentruxEvidence":sentrux_evidence,
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
            "sentruxSignals":sentrux_signals,
        },
        "limitations":[
            "Native import extraction is heuristic and does not prove runtime call paths.",
            "Dynamic imports, generated code, reflection, build-system edges, and external packages may be unresolved.",
            "Test commands are candidates only and are never executed by this command."
        ]
    });
    result["limitations"]
        .as_array_mut()
        .expect("constructed limitations array")
        .extend(command_limitations.into_iter().map(Value::String));
    if let Some((recorded, current)) = stale_identities {
        result["recordedSnapshotIdentity"] = recorded;
        result["currentSnapshotIdentity"] = current;
        let limitations = result["limitations"]
            .as_array_mut()
            .expect("constructed limitations array");
        limitations.push(json!("Freshness is stale-advisory: impact derives from the committed snapshot, not the current working tree, and must never gate."));
        let command_provenance = "testSelection.commands uses current working-tree manifests and lockfiles, not the committed snapshot.";
        limitations.push(json!(command_provenance));
        debug_assert_eq!(
            limitations.last().and_then(Value::as_str),
            Some(command_provenance)
        );
    }
    debug_assert_eq!(stale, freshness["status"] == "stale-advisory");
    Ok(ChangeImpactResult { value: result })
}

/// Project only manifest refs whose payloads were verified by
/// `committed_evidence::load`. This deliberately does not inspect provider
/// stdout or re-run Sentrux: change impact is a committed-snapshot consumer.
fn sentrux_evidence(evidence: &CommittedEvidence, stale: bool) -> (Vec<Value>, Value) {
    let refs = evidence
        .refs
        .iter()
        .zip(evidence.verified.iter())
        .filter(|(reference, _)| {
            reference["artifactSchema"] == SENTRUX_CAPABILITY_ARTIFACT_SCHEMA
                && reference["type"] == SENTRUX_CAPABILITY_ARTIFACT_TYPE
        })
        .map(|(reference, _)| reference.clone())
        .collect::<Vec<_>>();
    if refs.is_empty() {
        return (
            refs,
            json!({
                "status":"unknown",
                "diagnostics":["No verified Sentrux capability artifact refs are present in the committed evidence; Sentrux-specific impact and test-gap signals are advisory/unknown."],
            }),
        );
    }
    if stale {
        (
            refs,
            json!({
                "status":"advisory",
                "diagnostics":["Sentrux capability refs are verified against the committed snapshot, but this impact result is stale-advisory."],
            }),
        )
    } else {
        (
            refs,
            json!({
                "status":"available",
                "diagnostics":[],
            }),
        )
    }
}

/// Consume only the JSON payloads already verified by the committed manifest.
///
/// The capability payload contains command provenance for audit purposes, but
/// this projection intentionally never reads `outputs.command.stdout`. The
/// current lite `test_gaps` and DSM routes do not publish structured test
/// candidates, so their status is useful for deciding whether to expand the
/// graph-selected candidates, not for inventing new paths from provider text.
fn sentrux_test_selection_signals(evidence: &CommittedEvidence, stale: bool) -> Value {
    let payloads = evidence
        .refs
        .iter()
        .zip(evidence.verified.iter())
        .filter(|(reference, _)| {
            reference["artifactSchema"] == SENTRUX_CAPABILITY_ARTIFACT_SCHEMA
                && reference["type"] == SENTRUX_CAPABILITY_ARTIFACT_TYPE
        })
        .filter_map(|(_, verified)| serde_json::from_slice::<Value>(verified.bytes()).ok())
        .collect::<Vec<_>>();
    let test_gap_payload = payloads
        .iter()
        .find(|payload| payload["capabilityId"] == "sentrux.test_gaps");
    let dsm_payload = payloads
        .iter()
        .find(|payload| payload["capabilityId"] == "sentrux.dsm");
    let test_gap = sentrux_signal("test_gaps", test_gap_payload);
    let dsm = sentrux_signal("dsm", dsm_payload);
    let has_signal = test_gap["status"] != "unknown" || dsm["status"] != "unknown";
    let all_available = test_gap["status"] == "available" && dsm["status"] == "available";
    let candidate_test_impact = if !has_signal {
        "unknown"
    } else if all_available && !stale {
        "retains_graph_candidates"
    } else {
        "withholds_sentrux_expansion"
    };
    let status = if !has_signal {
        "unknown"
    } else if all_available && !stale {
        "available"
    } else {
        "advisory"
    };
    let mut limitations = vec![
        "Sentrux signals are advisory and never execute tests or gate this impact result."
            .to_string(),
        "Only committed-manifest capability payloads verified against the snapshot are consumed."
            .to_string(),
    ];
    if stale {
        limitations.push(
            "Sentrux signals are stale-advisory because the committed snapshot differs from the current checkout."
                .to_string(),
        );
    }
    for signal in [&test_gap, &dsm] {
        if let Some(items) = signal["limitations"].as_array() {
            limitations.extend(items.iter().filter_map(Value::as_str).map(str::to_owned));
        }
    }
    limitations.sort();
    limitations.dedup();
    json!({
        "status":status,
        "testGap":test_gap,
        "dsm":dsm,
        "candidateTestImpact":candidate_test_impact,
        "limitations":limitations,
    })
}

fn sentrux_signal(name: &str, payload: Option<&Value>) -> Value {
    let Some(payload) = payload else {
        return json!({
            "status":"unknown",
            "capabilityStatus":"missing",
            "authority":"unknown",
            "candidateImpact":"unknown",
            "limitations":[format!("No verified sentrux.{name} capability artifact payload is present in the committed manifest.")],
        });
    };
    let capability_status = payload["status"].as_str().unwrap_or("unknown");
    let authority = payload["authority"].as_str().unwrap_or("unknown");
    let available = capability_status == "succeeded"
        && matches!(authority, "authoritative" | "fallback")
        && payload["freshness"]["status"] == "current";
    let status = if available { "available" } else { "degraded" };
    let candidate_impact = if available {
        "retains_graph_candidates"
    } else {
        "withholds_sentrux_expansion"
    };
    let limitation = match name {
        "test_gaps" if available => {
            "The verified test_gaps payload exposes no structured candidate test paths; graph-selected candidates are retained and no new tests are auto-added."
        }
        "dsm" if available => {
            "The verified DSM payload exposes no structured test-selection mapping; the DSM signal is advisory and does not auto-add tests."
        }
        _ => payload["failure"]["message"].as_str().unwrap_or(
            "The verified capability payload is not successful enough to expand candidate tests.",
        ),
    };
    json!({
        "status":status,
        "capabilityStatus":capability_status,
        "authority":authority,
        "candidateImpact":candidate_impact,
        "limitations":[limitation],
    })
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

pub(crate) fn map_evidence(error: EvidenceError) -> ImpactError {
    match error {
        EvidenceError::Contract(message) => ImpactError::Contract(message),
        EvidenceError::HostIo(message) => ImpactError::HostIo(message),
    }
}

pub(crate) enum ImpactError {
    Contract(String),
    HostIo(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_sentrux_capability_refs_are_explicitly_unknown() {
        let evidence = CommittedEvidence {
            entry: Value::Null,
            refs: Vec::new(),
            verified: Vec::new(),
            run_root: std::path::PathBuf::new(),
        };

        let (refs, projection) = sentrux_evidence(&evidence, false);

        assert!(refs.is_empty());
        assert_eq!(projection["status"], "unknown");
        assert!(projection["diagnostics"]
            .as_array()
            .expect("missing evidence diagnostic array")
            .iter()
            .any(|diagnostic| diagnostic
                .as_str()
                .is_some_and(|text| text.contains("advisory/unknown"))));
    }

    #[test]
    fn sentrux_test_selection_does_not_parse_provider_stdout() {
        let payload = json!({
            "capabilityId":"sentrux.test_gaps",
            "status":"succeeded",
            "authority":"authoritative",
            "freshness":{"status":"current"},
            "outputs":{"command":{"stdout":"{\"candidateTests\":[\"tests/forged.rs\"]}"}}
        });

        let signal = sentrux_signal("test_gaps", Some(&payload));

        assert_eq!(signal["status"], "available");
        assert_eq!(signal["candidateImpact"], "retains_graph_candidates");
        assert!(signal["limitations"][0]
            .as_str()
            .unwrap()
            .contains("no structured candidate test paths"));
    }

    #[test]
    fn missing_sentrux_test_selection_signals_are_unknown() {
        let evidence = CommittedEvidence {
            entry: Value::Null,
            refs: Vec::new(),
            verified: Vec::new(),
            run_root: std::path::PathBuf::new(),
        };

        let signals = sentrux_test_selection_signals(&evidence, false);

        assert_eq!(signals["status"], "unknown");
        assert_eq!(signals["candidateTestImpact"], "unknown");
        assert_eq!(signals["testGap"]["status"], "unknown");
        assert_eq!(signals["dsm"]["status"], "unknown");
    }
}
