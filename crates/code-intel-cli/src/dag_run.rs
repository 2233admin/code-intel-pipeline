use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

use crate::artifact_ref::{self, ArtifactError};
use crate::capability::sha256_hex;
use crate::dag_coordinator::{
    Coordinator, DagSpec, Dispatch, DomainVerdict, EdgeSpec, ExecutionFailure, NodeExecutor,
    NodeOutcome, NodeSpec, RunOutcome, VerifiedArtifactRef,
};
use crate::execution_kernel::{self, RunError};
use crate::execution_policy::{ExecutionPolicy, RunProfile, WorkingTreePolicy};
use crate::snapshot;

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let cli = match Cli::parse(raw) {
        Ok(cli) => cli,
        Err(error) => {
            eprintln!("{error}");
            return 64;
        }
    };
    match execute_cli(cli) {
        Ok(result) => {
            println!(
                "{}",
                serde_json::to_string(&result.output).expect("run result serializes")
            );
            result.exit_code
        }
        Err(error) => {
            eprintln!("{}", error.message);
            error.exit_code
        }
    }
}

struct CliResult {
    output: Value,
    exit_code: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RunCommand {
    DagCoordinate,
    Execute,
}

struct Cli {
    command: RunCommand,
    repo: PathBuf,
    out: PathBuf,
    authority_root: Option<PathBuf>,
    final_name: Option<String>,
    manifest: Option<PathBuf>,
    max_concurrency: usize,
    policy: ExecutionPolicy,
    diagnosis_inputs: Option<PathBuf>,
    seed_artifact_root: Option<PathBuf>,
    session_evidence: Option<PathBuf>,
}

impl Cli {
    fn parse(raw: &[String]) -> Result<Self, String> {
        let command = match raw.first().map(String::as_str) {
            Some("dag-coordinate") => RunCommand::DagCoordinate,
            Some("execute") => RunCommand::Execute,
            _ => return Err(usage()),
        };
        let mut repo = None;
        let mut out = None;
        let mut authority_root = None;
        let mut final_name = None;
        let mut manifest = None;
        let mut max_concurrency = 2usize;
        let mut working_tree_policy = "explicit_overlay".to_string();
        let mut scopes = Vec::new();
        let mut diagnosis_inputs = None;
        let mut seed_artifact_root = None;
        let mut doctor_tool_path_prefix = None;
        let mut doctor_require_repowise = None;
        let mut doctor_require_understand = None;
        let mut profile = match command {
            RunCommand::DagCoordinate => RunProfile::Compatibility,
            RunCommand::Execute => RunProfile::Default,
        };
        let mut session_evidence = None;
        let mut index = 1;
        while index < raw.len() {
            let flag = raw[index].as_str();
            if !matches!(
                flag,
                "--repo"
                    | "--out"
                    | "--authority-root"
                    | "--final-name"
                    | "--manifest"
                    | "--profile"
                    | "--max-concurrency"
                    | "--working-tree-policy"
                    | "--scope"
                    | "--diagnosis-inputs"
                    | "--seed-artifact-root"
                    | "--doctor-tool-path-prefix"
                    | "--doctor-require-repowise"
                    | "--doctor-require-understand"
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
                "--authority-root" if authority_root.replace(PathBuf::from(value)).is_some() => {
                    return Err("duplicate --authority-root".into())
                }
                "--final-name" if final_name.replace(value.clone()).is_some() => {
                    return Err("duplicate --final-name".into())
                }
                "--manifest" if manifest.replace(PathBuf::from(value)).is_some() => {
                    return Err("duplicate --manifest".into())
                }
                "--profile" => {
                    if command != RunCommand::Execute {
                        return Err("--profile is available only for run execute".into());
                    }
                    profile = RunProfile::parse(value)?;
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
                "--doctor-require-repowise" => {
                    doctor_require_repowise = Some(parse_bool_flag(flag, value)?);
                }
                "--doctor-require-understand" => {
                    doctor_require_understand = Some(parse_bool_flag(flag, value)?);
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
        match command {
            RunCommand::DagCoordinate => {
                if authority_root.is_some() || final_name.is_some() {
                    return Err(
                        "--authority-root and --final-name are available only for run execute"
                            .into(),
                    );
                }
            }
            RunCommand::Execute => {
                if diagnosis_inputs.is_some() || seed_artifact_root.is_some() {
                    return Err(
                        "run execute does not accept diagnosis-only inputs; use run dag-coordinate for the non-authoritative compatibility primitive"
                            .into(),
                    );
                }
                let authority = authority_root
                    .as_ref()
                    .ok_or("run execute requires --authority-root")?;
                if !authority.is_dir() {
                    return Err(format!(
                        "authority root is not a directory: {}",
                        authority.display()
                    ));
                }
                if final_name.is_none() {
                    return Err("run execute requires --final-name".into());
                }
            }
        }
        let policy = ExecutionPolicy::for_profile(profile)
            .with_working_tree(WorkingTreePolicy::parse(&working_tree_policy)?, scopes)
            .with_doctor_overrides(
                doctor_require_repowise,
                doctor_require_understand,
                doctor_tool_path_prefix,
            );
        Ok(Self {
            command,
            repo,
            out: out.ok_or("--out is required")?,
            authority_root,
            final_name,
            manifest,
            max_concurrency,
            policy,
            diagnosis_inputs,
            seed_artifact_root,
            session_evidence,
        })
    }
}

fn usage() -> String {
    "usage: run <dag-coordinate|execute> --repo <repo-root> --out <run-staging-directory> [--authority-root <publication-root> --final-name <name>] [--profile <default|strict|offline>] [--manifest <integrations.json>] [--max-concurrency <n>] [--working-tree-policy <head_only|explicit_overlay>] [--scope <relative-path>]... [--session-evidence <session-evidence.json>] [--diagnosis-inputs <artifact-refs.json> --seed-artifact-root <root>] [--doctor-tool-path-prefix <directory>] [--doctor-require-repowise <true|false>] [--doctor-require-understand <true|false>]".into()
}

fn parse_bool_flag(flag: &str, value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("{flag} must be true or false")),
    }
}

pub(crate) struct DagExecutionRequest {
    pub(crate) repo: PathBuf,
    pub(crate) out: PathBuf,
    pub(crate) manifest: Option<PathBuf>,
    pub(crate) max_concurrency: usize,
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

fn execute_cli(cli: Cli) -> Result<CliResult, RunError> {
    match cli.command {
        RunCommand::DagCoordinate => {
            let result = execute_dag(DagExecutionRequest {
                repo: cli.repo,
                out: cli.out,
                manifest: cli.manifest,
                max_concurrency: cli.max_concurrency,
                policy: cli.policy,
                diagnosis_inputs: cli.diagnosis_inputs,
                seed_artifact_root: cli.seed_artifact_root,
                session_evidence: cli.session_evidence,
            })?;
            Ok(CliResult {
                output: result.manifest,
                exit_code: result.outcome.exit_code(),
            })
        }
        RunCommand::Execute => {
            let result = execution_kernel::execute(execution_kernel::RunRequest {
                repo: cli.repo,
                staging_root: cli.out,
                authority_root: cli
                    .authority_root
                    .expect("validated execute authority root"),
                final_name: cli.final_name.expect("validated execute final name"),
                manifest: cli.manifest,
                max_concurrency: cli.max_concurrency,
                policy: cli.policy,
                session_evidence: cli.session_evidence,
            })?;
            let exit_code = result.exit_code();
            Ok(CliResult {
                output: result.to_json(),
                exit_code,
            })
        }
    }
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
    };
    let manifest = Coordinator::new(spec)
        .map_err(|error| RunError::contract(error.to_string()))?
        .run_to_completion(&executor)
        .map_err(|error| RunError::contract(error.to_string()))?;
    let exit_code = manifest.outcome.exit_code();
    let manifest_json = manifest.to_json();
    persist_commit_handoff(&run_root, &manifest_json, &snapshot_identity)?;
    debug_assert_eq!(exit_code, manifest.outcome.exit_code());
    Ok(DagExecutionResult {
        manifest: manifest_json,
        outcome: manifest.outcome,
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
