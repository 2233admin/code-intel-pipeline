use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

use crate::artifact_ref::{self, ArtifactError};
use crate::capability::sha256_hex;
use crate::dag_coordinator::{
    Coordinator, DagSpec, Dispatch, DomainVerdict, EdgeSpec, ExecutionFailure, NodeExecutor,
    NodeOutcome, NodeSpec, VerifiedArtifactRef,
};
use crate::snapshot;

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let cli = match Cli::parse(raw) {
        Ok(cli) => cli,
        Err(error) => {
            eprintln!("{error}");
            return 64;
        }
    };
    match execute(cli) {
        Ok(manifest) => {
            println!(
                "{}",
                serde_json::to_string(&manifest).expect("run manifest serializes")
            );
            match manifest["outcome"].as_str() {
                Some("completed") => 0,
                Some("domain_failed") => 10,
                Some("domain_unknown") => 20,
                Some("process_failed" | "incomplete") => 70,
                _ => 70,
            }
        }
        Err(error) => {
            eprintln!("{}", error.message);
            error.exit_code
        }
    }
}

struct Cli {
    repo: PathBuf,
    out: PathBuf,
    manifest: Option<PathBuf>,
    max_concurrency: usize,
    working_tree_policy: String,
    scopes: Vec<String>,
    diagnosis_inputs: Option<PathBuf>,
    seed_artifact_root: Option<PathBuf>,
    doctor_tool_path_prefix: Option<PathBuf>,
    session_evidence: Option<PathBuf>,
}

impl Cli {
    fn parse(raw: &[String]) -> Result<Self, String> {
        if raw.first().map(String::as_str) != Some("dag-coordinate") {
            return Err("usage: run dag-coordinate --repo <repo-root> --out <run-staging-directory> [--manifest <integrations.json>] [--max-concurrency <n>] [--working-tree-policy <head_only|explicit_overlay>] [--scope <relative-path>]... [--session-evidence <session-evidence.json>] [--diagnosis-inputs <artifact-refs.json> --seed-artifact-root <root>] [--doctor-tool-path-prefix <directory>]".into());
        }
        let mut repo = None;
        let mut out = None;
        let mut manifest = None;
        let mut max_concurrency = 2usize;
        let mut working_tree_policy = "explicit_overlay".to_string();
        let mut scopes = Vec::new();
        let mut diagnosis_inputs = None;
        let mut seed_artifact_root = None;
        let mut doctor_tool_path_prefix = None;
        let mut session_evidence = None;
        let mut index = 1;
        while index < raw.len() {
            let flag = raw[index].as_str();
            if !matches!(
                flag,
                "--repo"
                    | "--out"
                    | "--manifest"
                    | "--max-concurrency"
                    | "--working-tree-policy"
                    | "--scope"
                    | "--diagnosis-inputs"
                    | "--seed-artifact-root"
                    | "--doctor-tool-path-prefix"
                    | "--session-evidence"
            ) {
                return Err(format!("unknown DAG run argument: {flag}"));
            }
            let value = raw
                .get(index + 1)
                .filter(|value| !value.is_empty() && !value.starts_with("--"))
                .ok_or_else(|| format!("{flag} requires one value"))?;
            match flag {
                "--repo" if repo.replace(PathBuf::from(value)).is_some() => {
                    return Err("duplicate --repo".into())
                }
                "--out" if out.replace(PathBuf::from(value)).is_some() => {
                    return Err("duplicate --out".into())
                }
                "--manifest" if manifest.replace(PathBuf::from(value)).is_some() => {
                    return Err("duplicate --manifest".into())
                }
                "--max-concurrency" => {
                    max_concurrency = value
                        .parse::<usize>()
                        .map_err(|_| "--max-concurrency must be an integer".to_string())?;
                }
                "--working-tree-policy" => working_tree_policy = value.clone(),
                "--scope" => scopes.push(value.clone()),
                "--diagnosis-inputs"
                    if diagnosis_inputs.replace(PathBuf::from(value)).is_some() =>
                {
                    return Err("duplicate --diagnosis-inputs".into())
                }
                "--seed-artifact-root"
                    if seed_artifact_root.replace(PathBuf::from(value)).is_some() =>
                {
                    return Err("duplicate --seed-artifact-root".into())
                }
                "--doctor-tool-path-prefix"
                    if doctor_tool_path_prefix
                        .replace(PathBuf::from(value))
                        .is_some() =>
                {
                    return Err("duplicate --doctor-tool-path-prefix".into())
                }
                "--session-evidence"
                    if session_evidence.replace(PathBuf::from(value)).is_some() =>
                {
                    return Err("duplicate --session-evidence".into())
                }
                _ => {}
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
        if scopes.is_empty() {
            scopes.push(".".into());
        }
        if diagnosis_inputs.is_some() != seed_artifact_root.is_some() {
            return Err(
                "--diagnosis-inputs and --seed-artifact-root must be provided together".into(),
            );
        }
        if diagnosis_inputs.is_some() && doctor_tool_path_prefix.is_some() {
            return Err(
                "--doctor-tool-path-prefix is valid only for the default DAG containing doctor"
                    .into(),
            );
        }
        if diagnosis_inputs.is_some() && session_evidence.is_some() {
            return Err("--session-evidence is valid only for the default analysis DAG".into());
        }
        if let Some(path) = &session_evidence {
            if !path.is_file() {
                return Err(format!(
                    "session evidence path is not a file: {}",
                    path.display()
                ));
            }
        }
        if let Some(prefix) = &mut doctor_tool_path_prefix {
            if !prefix.is_dir() {
                return Err(format!(
                    "--doctor-tool-path-prefix is not a directory: {}",
                    prefix.display()
                ));
            }
            if prefix.is_relative() {
                *prefix = std::env::current_dir()
                    .map_err(|error| format!("resolve current directory: {error}"))?
                    .join(&*prefix);
            }
        }
        Ok(Self {
            repo,
            out: out.ok_or("--out is required")?,
            manifest,
            max_concurrency,
            working_tree_policy,
            scopes,
            diagnosis_inputs,
            seed_artifact_root,
            doctor_tool_path_prefix,
            session_evidence,
        })
    }
}

struct RunError {
    exit_code: i32,
    message: String,
}

impl RunError {
    fn contract(message: impl Into<String>) -> Self {
        Self {
            exit_code: 65,
            message: message.into(),
        }
    }

    fn io(message: impl Into<String>) -> Self {
        Self {
            exit_code: 74,
            message: message.into(),
        }
    }
}

fn execute(cli: Cli) -> Result<Value, RunError> {
    let snapshot_document =
        snapshot::build_for_dag(&cli.repo, &cli.working_tree_policy, &cli.scopes)
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
    let required = if diagnosis_mode {
        vec!["diagnosis.hospital"]
    } else {
        vec![
            "repo.snapshot",
            "doctor",
            "inventory.rg",
            "evidence.native-code",
            "provider.graph-adapt",
            "provider.sentrux-adapt",
            "diagnosis.hospital",
        ]
    };
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
            NodeSpec::new(
                "evidence.graph",
                "provider.graph-adapt",
                request_identity("provider.graph-adapt"),
            ),
            NodeSpec::new(
                "evidence.sentrux",
                "provider.sentrux-adapt",
                request_identity("provider.sentrux-adapt"),
            ),
            NodeSpec::new(
                "diagnosis.hospital",
                "diagnosis.hospital",
                request_identity("diagnosis.hospital"),
            ),
        ];
        let mut edges = vec![
            EdgeSpec::new("repo.snapshot", "doctor"),
            EdgeSpec::new("repo.snapshot", "inventory.rg"),
            EdgeSpec::new("inventory.rg", "evidence.native-code"),
            EdgeSpec::new("repo.snapshot", "evidence.graph"),
            EdgeSpec::new("repo.snapshot", "evidence.sentrux"),
            EdgeSpec::new("evidence.graph", "diagnosis.hospital"),
            EdgeSpec::new("evidence.sentrux", "diagnosis.hospital"),
        ];
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
        doctor_tool_path_prefix: cli.doctor_tool_path_prefix,
    };
    let manifest = Coordinator::new(spec)
        .map_err(|error| RunError::contract(error.to_string()))?
        .run_to_completion(&executor)
        .map_err(|error| RunError::contract(error.to_string()))?;
    let manifest = manifest.to_json();
    persist_commit_handoff(&run_root, &manifest, &snapshot_identity)?;
    Ok(manifest)
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
    doctor_tool_path_prefix: Option<PathBuf>,
}

impl NodeExecutor for CapabilityEnvelopeExecutor {
    fn execute(&self, dispatch: Dispatch) -> NodeOutcome {
        match self.execute_node(dispatch) {
            Ok(outcome) => outcome,
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
        let mut options = match dispatch.capability.as_str() {
            "diagnosis.hospital" => json!({}),
            "doctor" => json!({"repoPath":self.repo,"manifestPath":self.registry_path}),
            _ => json!({"repoPath":self.repo}),
        };
        if dispatch.capability == "doctor" {
            if let Some(prefix) = &self.doctor_tool_path_prefix {
                options["toolPathPrefix"] = json!(prefix);
            }
        }
        if dispatch.capability == "provider.sentrux-adapt" {
            if let Some(prefix) = &self.doctor_tool_path_prefix {
                options["toolPathPrefix"] = json!(prefix);
            }
        }
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
            "effectPolicy":{"allowedEffects":declaration["allowedEffects"]}
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
        let output = command.output().map_err(|error| {
            (
                ExecutionFailure::Unavailable,
                format!("launch A01 capability executor: {error}"),
            )
        })?;
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
        let result: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
            (
                ExecutionFailure::Contract,
                format!("A01 executor emitted invalid result JSON: {error}"),
            )
        })?;
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
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("orchestration")
        .join("integrations.json")
}
