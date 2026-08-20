use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::{json, Value};

use crate::artifact_ref::{self, ArtifactError};
use crate::budget::Budget;
use crate::budget_dispatch;
use crate::capability::{self, sha256_hex};
use crate::dag_coordinator::{
    Coordinator, DagSpec, Dispatch, DomainVerdict, EdgeSpec, ExecutionFailure, NodeExecutor,
    NodeOutcome, NodeSpec, RunOutcome, VerifiedArtifactRef,
};
use crate::execution_policy::ExecutionPolicy;
use crate::node_timeout::{self, NodeTimeoutError};
use crate::run_error::RunError;
use crate::snapshot;

pub(crate) struct DagExecutionRequest {
    pub(crate) repo: PathBuf,
    pub(crate) out: PathBuf,
    pub(crate) manifest: Option<PathBuf>,
    pub(crate) node_timeout: Duration,
    pub(crate) max_concurrency: usize,
    pub(crate) budget: Budget,
    pub(crate) policy: ExecutionPolicy,
    pub(crate) diagnosis_inputs: Option<PathBuf>,
    pub(crate) seed_artifact_root: Option<PathBuf>,
    pub(crate) session_evidence: Option<PathBuf>,
}

pub(crate) struct DagExecutionResult {
    pub(crate) manifest: Value,
    pub(crate) outcome: RunOutcome,
    pub(crate) run_root: PathBuf,
}

pub(crate) fn execute_dag(cli: DagExecutionRequest) -> Result<DagExecutionResult, RunError> {
    let snapshot_document =
        snapshot::build_for_dag(&cli.repo, cli.policy.working_tree(), cli.policy.scopes())
            .map_err(RunError::contract)?;
    let snapshot_identity = snapshot_document["snapshot"]["identity"]
        .as_str()
        .expect("A02 snapshot has validated identity")
        .to_string();
    let registry_path = cli.manifest.unwrap_or_else(default_registry);
    let diagnosis_mode = cli.diagnosis_inputs.is_some();
    let (seeded_inputs, seed_artifact_root) = if let (Some(input_path), Some(artifact_root)) =
        (&cli.diagnosis_inputs, &cli.seed_artifact_root)
    {
        let raw = fs::read(input_path).map_err(|error| {
            RunError::io(format!("read seeded diagnosis Artifact Refs: {error}"))
        })?;
        let refs: Value = serde_json::from_slice(&raw).map_err(|error| {
            RunError::contract(format!("parse seeded diagnosis Artifact Refs: {error}"))
        })?;
        let verified = artifact_ref::verify_inputs(&refs, Some(artifact_root), &snapshot_identity)
            .map_err(|error| match error {
                ArtifactError::Contract(message) => RunError::contract(message),
                ArtifactError::Io(message) => RunError::io(message),
            })?;
        let ref_values = refs
            .as_array()
            .ok_or_else(|| RunError::contract("seeded diagnosis Artifact Refs must be an array"))?;
        if ref_values.is_empty() {
            return Err(RunError::contract(
                "seeded diagnosis Artifact Refs must not be empty",
            ));
        }
        let converted = ref_values
            .iter()
            .zip(verified.iter())
            .map(|(artifact, verified)| {
                VerifiedArtifactRef::from_a03(
                    artifact["path"]
                        .as_str()
                        .expect("A03 verified Artifact Ref path"),
                    verified,
                )
                .map_err(|error| RunError::contract(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        (converted, Some(artifact_root.clone()))
    } else {
        (Vec::new(), None)
    };
    let registry = read_registry(&registry_path)?;
    let declarations = declarations(&registry)?;
    let mut required = if diagnosis_mode {
        vec!["diagnosis.hospital"]
    } else {
        vec![
            "repo.snapshot",
            "doctor",
            "inventory.rg",
            "evidence.native-code",
        ]
    };
    if !diagnosis_mode {
        if cli.policy.capability_enabled("provider.graph-adapt") {
            required.push("provider.graph-adapt");
        }
        if cli.policy.capability_enabled("provider.sentrux-adapt") {
            required.push("provider.sentrux-adapt");
        }
        if cli.policy.capability_enabled("provider.codenexus-adapt") {
            required.push("provider.codenexus-adapt");
        }
        if cli.policy.provider_diagnosis_enabled() {
            required.push("diagnosis.hospital");
        }
    }
    for required in required {
        if !declarations.contains_key(required) {
            return Err(RunError::contract(format!(
                "DAG capability is not registered: {required}"
            )));
        }
    }
    fs::create_dir(&cli.out).map_err(|error| {
        RunError::io(format!(
            "exclusive DAG staging create {}: {error}",
            cli.out.display()
        ))
    })?;
    let session_inputs = match cli.session_evidence.as_deref() {
        Some(path) => stage_session_evidence(&cli.out, path, &snapshot_identity)?,
        None => Vec::new(),
    };
    let request_identity = |capability: &str| format!("{capability}:{snapshot_identity}");
    let spec = if diagnosis_mode {
        DagSpec::new(
            &snapshot_identity,
            1,
            vec![
                NodeSpec::new(
                    "seed.admitted-evidence",
                    "a03.seeded-inputs",
                    request_identity("a03.seeded-inputs"),
                ),
                NodeSpec::new(
                    "diagnosis.hospital",
                    "diagnosis.hospital",
                    request_identity("diagnosis.hospital"),
                ),
            ],
            vec![EdgeSpec::new(
                "seed.admitted-evidence",
                "diagnosis.hospital",
            )],
        )
    } else {
        let mut nodes = vec![
            NodeSpec::new(
                "repo.snapshot",
                "repo.snapshot",
                request_identity("repo.snapshot"),
            ),
            NodeSpec::new("doctor", "doctor", request_identity("doctor")),
            NodeSpec::new(
                "inventory.rg",
                "inventory.rg",
                request_identity("inventory.rg"),
            ),
            NodeSpec::new(
                "evidence.native-code",
                "evidence.native-code",
                request_identity("evidence.native-code"),
            ),
        ];
        let mut edges = vec![
            EdgeSpec::new("repo.snapshot", "doctor"),
            EdgeSpec::new("repo.snapshot", "inventory.rg"),
            EdgeSpec::new("inventory.rg", "evidence.native-code"),
        ];
        if cli.policy.capability_enabled("provider.graph-adapt") {
            nodes.push(NodeSpec::new(
                "evidence.graph",
                "provider.graph-adapt",
                request_identity("provider.graph-adapt"),
            ));
            edges.push(EdgeSpec::new("repo.snapshot", "evidence.graph"));
            edges.push(EdgeSpec::new("evidence.graph", "diagnosis.hospital"));
        }
        if cli.policy.capability_enabled("provider.sentrux-adapt") {
            nodes.push(NodeSpec::new(
                "evidence.sentrux",
                "provider.sentrux-adapt",
                request_identity("provider.sentrux-adapt"),
            ));
            edges.push(EdgeSpec::new("repo.snapshot", "evidence.sentrux"));
            edges.push(EdgeSpec::new("evidence.sentrux", "diagnosis.hospital"));
        }
        if cli.policy.capability_enabled("provider.codenexus-adapt") {
            nodes.push(NodeSpec::new(
                "evidence.codenexus",
                "provider.codenexus-adapt",
                request_identity("provider.codenexus-adapt"),
            ));
            edges.push(EdgeSpec::new("repo.snapshot", "evidence.codenexus"));
            edges.push(EdgeSpec::new("evidence.codenexus", "diagnosis.hospital"));
        }
        if cli.policy.provider_diagnosis_enabled() {
            nodes.push(NodeSpec::new(
                "diagnosis.hospital",
                "diagnosis.hospital",
                request_identity("diagnosis.hospital"),
            ));
        }
        if !session_inputs.is_empty() {
            nodes.push(NodeSpec::new(
                "verification.session-evidence",
                "a03.session-evidence",
                request_identity("a03.session-evidence"),
            ));
            edges.push(EdgeSpec::new(
                "repo.snapshot",
                "verification.session-evidence",
            ));
        }
        DagSpec::new(&snapshot_identity, cli.max_concurrency, nodes, edges)
    };
    let run_root = cli.out.clone();
    let executor = CapabilityEnvelopeExecutor {
        binary: std::env::current_exe()
            .map_err(|error| RunError::io(format!("locate code-intel executable: {error}")))?,
        repo: cli.repo,
        run_root: run_root.clone(),
        registry_path,
        snapshot: snapshot_document["snapshot"].clone(),
        declarations,
        seeded_inputs,
        session_inputs,
        seed_artifact_root,
        policy: cli.policy,
        node_timeout: cli.node_timeout,
    };
    let budgeted = budget_dispatch::run_to_completion(
        Coordinator::new(spec).map_err(|error| RunError::contract(error.to_string()))?,
        &executor,
        cli.budget,
    )
    .map_err(|error| RunError::contract(error.to_string()))?;
    let outcome = budgeted.manifest.outcome;
    let exit_code = outcome.exit_code();
    let manifest_json = budgeted.manifest_json;
    persist_commit_handoff(&run_root, &manifest_json, &snapshot_identity)?;
    debug_assert_eq!(exit_code, outcome.exit_code());
    Ok(DagExecutionResult {
        manifest: manifest_json,
        outcome,
        run_root,
    })
}

fn stage_session_evidence(
    run_root: &Path,
    source: &Path,
    snapshot_identity: &str,
) -> Result<Vec<VerifiedArtifactRef>, RunError> {
    let metadata = fs::metadata(source)
        .map_err(|error| RunError::io(format!("inspect session evidence: {error}")))?;
    if !metadata.is_file() || metadata.len() > 128 * 1024 * 1024 {
        return Err(RunError::contract(
            "session evidence must be a regular file no larger than 128 MiB",
        ));
    }
    let bytes = fs::read(source)
        .map_err(|error| RunError::io(format!("read session evidence: {error}")))?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| RunError::contract(format!("parse session evidence: {error}")))?;
    crate::session_evidence::validate_artifact_value(&value).map_err(RunError::contract)?;
    if value["snapshot"]["identity"] != snapshot_identity {
        return Err(RunError::contract(
            "session evidence snapshot does not match the A09 run snapshot",
        ));
    }
    let relative = "verification.session-evidence/session-evidence.json";
    let directory = run_root.join("verification.session-evidence");
    fs::create_dir(&directory)
        .map_err(|error| RunError::io(format!("create session evidence staging: {error}")))?;
    fs::write(run_root.join(relative), &bytes)
        .map_err(|error| RunError::io(format!("stage session evidence: {error}")))?;
    let reference = json!({
        "schema":"code-intel-artifact-ref.v1",
        "artifactSchema":"code-intel-session-evidence.v1",
        "type":"verification.session-evidence",
        "path":relative,
        "sha256":sha256_hex(&bytes),
        "consumedSnapshotIdentity":snapshot_identity,
    });
    let refs = json!([reference]);
    let verified =
        artifact_ref::verify_inputs(&refs, Some(run_root), snapshot_identity).map_err(|error| {
            match error {
                ArtifactError::Contract(message) => RunError::contract(message),
                ArtifactError::Io(message) => RunError::io(message),
            }
        })?;
    let converted = VerifiedArtifactRef::from_a03(relative, &verified[0])
        .map_err(|error| RunError::contract(error.to_string()))?;
    Ok(vec![converted])
}

fn persist_commit_handoff(
    run_root: &Path,
    manifest: &Value,
    snapshot_identity: &str,
) -> Result<(), RunError> {
    let manifest_bytes = serde_json::to_vec(manifest)
        .map_err(|error| RunError::contract(format!("serialize terminal run manifest: {error}")))?;
    let manifest_name = "run-manifest.json";
    fs::write(run_root.join(manifest_name), &manifest_bytes)
        .map_err(|error| RunError::io(format!("write terminal run manifest: {error}")))?;
    let manifest_ref = json!({
        "schema":"code-intel-artifact-ref.v1",
        "artifactSchema":"code-intel-run-manifest.v1",
        "type":"run.manifest",
        "path":manifest_name,
        "sha256":sha256_hex(&manifest_bytes),
        "consumedSnapshotIdentity":snapshot_identity,
    });
    fs::write(
        run_root.join("run-manifest-ref.json"),
        serde_json::to_vec(&manifest_ref).expect("run manifest Artifact Ref serializes"),
    )
    .map_err(|error| RunError::io(format!("write run manifest Artifact Ref: {error}")))
}

struct CapabilityEnvelopeExecutor {
    binary: PathBuf,
    repo: PathBuf,
    run_root: PathBuf,
    registry_path: PathBuf,
    snapshot: Value,
    declarations: BTreeMap<String, Value>,
    seeded_inputs: Vec<VerifiedArtifactRef>,
    session_inputs: Vec<VerifiedArtifactRef>,
    seed_artifact_root: Option<PathBuf>,
    policy: ExecutionPolicy,
    node_timeout: Duration,
}

impl NodeExecutor for CapabilityEnvelopeExecutor {
    fn execute(&self, dispatch: Dispatch) -> NodeOutcome {
        let capability = dispatch.capability.clone();
        match self.execute_node(dispatch) {
            Ok(outcome) => outcome,
            Err((ExecutionFailure::Unavailable, _))
                if self
                    .policy
                    .capability_requirement(&capability)
                    .is_some_and(|requirement| !requirement.is_required()) =>
            {
                NodeOutcome::success(DomainVerdict::NotApplicable, Vec::new())
            }
            Err((failure, message)) => NodeOutcome::process_failure(failure, message),
        }
    }
}

impl CapabilityEnvelopeExecutor {
    fn execute_node(&self, dispatch: Dispatch) -> Result<NodeOutcome, (ExecutionFailure, String)> {
        if dispatch.capability == "a03.seeded-inputs" {
            if self.seeded_inputs.is_empty() {
                return Err((
                    ExecutionFailure::Contract,
                    "seeded diagnosis path requires at least one A03-verified Artifact Ref".into(),
                ));
            }
            return Ok(NodeOutcome::success(
                DomainVerdict::Pass,
                self.seeded_inputs.clone(),
            ));
        }
        if dispatch.capability == "a03.session-evidence" {
            if self.session_inputs.len() != 1 {
                return Err((
                    ExecutionFailure::Contract,
                    "optional session verification requires one A03-verified artifact".into(),
                ));
            }
            return Ok(NodeOutcome::success(
                DomainVerdict::Pass,
                self.session_inputs.clone(),
            ));
        }
        let declaration = self.declarations.get(&dispatch.capability).ok_or_else(|| {
            (
                ExecutionFailure::Contract,
                format!("unregistered DAG capability: {}", dispatch.capability),
            )
        })?;
        let node_out = self.run_root.join(&dispatch.node_id);
        let request_path = self
            .run_root
            .join(format!("{}.request.json", dispatch.node_id));
        let options =
            self.policy
                .capability_options(&dispatch.capability, &self.repo, &self.registry_path);
        let inputs = if dispatch.capability == "diagnosis.hospital" {
            dispatch
                .inputs
                .iter()
                .filter(|reference| reference.artifact_type() == "evidence.admission")
                .map(VerifiedArtifactRef::to_json)
                .collect::<Vec<_>>()
        } else {
            dispatch
                .inputs
                .iter()
                .map(VerifiedArtifactRef::to_json)
                .collect::<Vec<_>>()
        };
        let request = json!({
            "schema":"code-intel-capability-request.v1",
            "capability":dispatch.capability,
            "contractVersion":1,
            "implementation":declaration["implementation"],
            "snapshot":self.snapshot,
            "options":options,
            "inputs":inputs,
            "effectPolicy":{"allowedEffects":self.policy.allowed_effects(declaration)}
        });
        fs::write(
            &request_path,
            serde_json::to_vec(&request).expect("A01 request serializes"),
        )
        .map_err(|error| {
            (
                ExecutionFailure::Io,
                format!("write DAG capability request: {error}"),
            )
        })?;
        let mut command = Command::new(&self.binary);
        command
            .args(["capability", "exec", &dispatch.capability, "--request"])
            .arg(&request_path)
            .arg("--out")
            .arg(&node_out)
            .arg("--artifact-root")
            .arg(if dispatch.capability == "diagnosis.hospital" {
                self.seed_artifact_root.as_ref().unwrap_or(&self.run_root)
            } else {
                &self.run_root
            })
            .arg("--manifest")
            .arg(&self.registry_path);
        let output = match node_timeout::run_with_timeout(command, self.node_timeout) {
            Ok(output) => output,
            Err(NodeTimeoutError::TimedOut { elapsed, limit }) => {
                return Ok(NodeOutcome::timeout(format!(
                    "node '{}' timed out; configured timeout limit: {}ms; actual elapsed: {}ms",
                    dispatch.node_id,
                    limit.as_millis(),
                    elapsed.as_millis(),
                )));
            }
            Err(NodeTimeoutError::Io(error)) => {
                return Err((
                    ExecutionFailure::Unavailable,
                    format!("launch A01 capability executor: {error}"),
                ))
            }
        };
        fs::write(
            self.run_root
                .join(format!("{}.result.json", dispatch.node_id)),
            &output.stdout,
        )
        .map_err(|error| {
            (
                ExecutionFailure::Io,
                format!("persist A01 result envelope: {error}"),
            )
        })?;
        let result: Value = match serde_json::from_slice(&output.stdout) {
            Ok(result) => result,
            Err(error) => {
                // A child that dies without writing a result envelope reaches
                // this branch with empty stdout, and "EOF at line 1 column 0"
                // says nothing about the death that produced it. Report the
                // child's exit status and stderr alongside the parse error so
                // a killed or crashed executor is distinguishable from one
                // that wrote malformed JSON (the diagnosability gap of issue
                // #123).
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stderr: String = stderr.trim().chars().take(512).collect();
                let evidence = if stderr.is_empty() {
                    "no stderr output".to_string()
                } else {
                    format!("stderr: {stderr}")
                };
                return Err(if output.status.success() {
                    (
                        ExecutionFailure::Contract,
                        format!("A01 executor emitted invalid result JSON: {error}; {evidence}"),
                    )
                } else {
                    (
                        failure_for_exit(output.status.code()),
                        format!(
                            "A01 capability executor exited without a result envelope: {}; result parse error: {error}; {evidence}",
                            output.status
                        ),
                    )
                });
            }
        };
        let result_verdict = result["verdict"].as_str().unwrap_or("");
        if !output.status.success() && result_verdict != "fail" {
            return Err((
                failure_for_exit(output.status.code()),
                diagnostics(&result, &output.stderr),
            ));
        }
        if result["status"] != "completed" || !matches!(result_verdict, "pass" | "fail") {
            return Err((
                ExecutionFailure::Contract,
                "A01 success result has invalid status/verdict".into(),
            ));
        }
        let mut rebased = result["artifacts"].clone();
        for artifact in rebased.as_array_mut().ok_or_else(|| {
            (
                ExecutionFailure::Contract,
                "A01 result artifacts must be an array".to_string(),
            )
        })? {
            let relative = artifact["path"].as_str().ok_or_else(|| {
                (
                    ExecutionFailure::Contract,
                    "A01 Artifact Ref path is missing".to_string(),
                )
            })?;
            artifact["path"] = json!(format!("{}/{relative}", dispatch.node_id));
        }
        let expected_snapshot = self.snapshot["identity"]
            .as_str()
            .expect("A02 snapshot identity");
        let verified =
            artifact_ref::verify_inputs(&rebased, Some(&self.run_root), expected_snapshot)
                .map_err(map_artifact_error)?;
        let refs = rebased
            .as_array()
            .expect("validated Artifact Ref array")
            .iter()
            .zip(verified.iter())
            .map(|(artifact, verified)| {
                VerifiedArtifactRef::from_a03(
                    artifact["path"].as_str().expect("validated path"),
                    verified,
                )
                .map_err(|error| (ExecutionFailure::Contract, error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let domain_verdict = match result["domainVerdict"].as_str() {
            Some("pass") => DomainVerdict::Pass,
            Some("fail") => DomainVerdict::Fail,
            Some("unknown") => DomainVerdict::Unknown,
            Some("not_applicable") => DomainVerdict::NotApplicable,
            other => {
                return Err((
                    ExecutionFailure::Contract,
                    format!("A01 result has invalid domainVerdict: {other:?}"),
                ))
            }
        };
        if result_verdict == "fail" {
            return Ok(NodeOutcome::domain_fail_with_artifacts(
                diagnostics(&result, &output.stderr),
                refs,
            ));
        }
        Ok(NodeOutcome::success(domain_verdict, refs))
    }
}

fn map_artifact_error(error: ArtifactError) -> (ExecutionFailure, String) {
    match error {
        ArtifactError::Contract(message) => (ExecutionFailure::Contract, message),
        ArtifactError::Io(message) => (ExecutionFailure::Io, message),
    }
}

fn failure_for_exit(code: Option<i32>) -> ExecutionFailure {
    match code {
        Some(64 | 65) => ExecutionFailure::Contract,
        Some(69) => ExecutionFailure::Unavailable,
        Some(74) => ExecutionFailure::Io,
        _ => ExecutionFailure::Internal,
    }
}

fn diagnostics(result: &Value, stderr: &[u8]) -> String {
    result["diagnostics"]
        .as_array()
        .and_then(|values| values.first())
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let message = String::from_utf8_lossy(stderr).trim().to_string();
            (!message.is_empty()).then_some(message)
        })
        .unwrap_or_else(|| "capability execution failed without diagnostic".into())
}

fn read_registry(path: &Path) -> Result<Value, RunError> {
    let bytes = fs::read(path)
        .map_err(|error| RunError::io(format!("read DAG capability registry: {error}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| RunError::contract(format!("parse DAG capability registry: {error}")))
}

fn declarations(registry: &Value) -> Result<BTreeMap<String, Value>, RunError> {
    let integrations = registry["integrations"]
        .as_array()
        .ok_or_else(|| RunError::contract("registry integrations must be an array"))?;
    let mut declarations = BTreeMap::new();
    for integration in integrations {
        let Some(declaration) = integration.get("capabilityDeclaration") else {
            continue;
        };
        let id = declaration["id"]
            .as_str()
            .ok_or_else(|| RunError::contract("registered capability declaration lacks id"))?;
        if declarations
            .insert(id.to_string(), declaration.clone())
            .is_some()
        {
            return Err(RunError::contract(format!(
                "duplicate registered capability declaration: {id}"
            )));
        }
    }
    Ok(declarations)
}

fn default_registry() -> PathBuf {
    capability::discover_manifest(None).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("orchestration")
            .join("integrations.json")
    })
}
