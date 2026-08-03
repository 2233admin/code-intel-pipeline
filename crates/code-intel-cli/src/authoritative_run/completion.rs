pub(super) fn bind_repository_iteration(
    run_root: &std::path::Path,
    manifest: &mut serde_json::Value,
    repository_key: &str,
    publication_name: &str,
) -> Result<(), crate::run_error::RunError> {
    let run_identity = manifest["runIdentity"].clone();
    let snapshot_identity = manifest["snapshotIdentity"].clone();
    let nodes = manifest["nodes"].as_object_mut().ok_or_else(|| {
        crate::run_error::RunError::contract("terminal run manifest nodes are invalid")
    })?;
    if nodes.contains_key("repository.iteration") {
        return Err(crate::run_error::RunError::contract(
            "DAG output cannot predeclare authoritative repository iteration provenance",
        ));
    }
    let payload = serde_json::json!({
        "schema":crate::artifact_ref::REPOSITORY_ITERATION_SCHEMA,
        "purpose":crate::artifact_ref::REPOSITORY_ITERATION_PURPOSE,
        "runIdentity":run_identity,
        "snapshotIdentity":snapshot_identity.clone(),
        "repositoryKey":repository_key,
        "publicationName":publication_name,
        "producer":{
            "component":crate::artifact_ref::REPOSITORY_ITERATION_PRODUCER_COMPONENT,
            "contract":crate::artifact_ref::REPOSITORY_ITERATION_PRODUCER_CONTRACT,
            "version":crate::artifact_ref::REPOSITORY_ITERATION_PRODUCER_VERSION,
        }
    });
    let payload_bytes = serde_json::to_vec(&payload).map_err(|error| {
        crate::run_error::RunError::contract(format!(
            "serialize repository iteration provenance: {error}"
        ))
    })?;
    let relative = "repository.iteration/provenance.json";
    let directory = run_root.join("repository.iteration");
    if directory.exists() {
        return Err(crate::run_error::RunError::contract(
            "repository iteration provenance path is already occupied",
        ));
    }
    std::fs::create_dir(&directory).map_err(|error| {
        crate::run_error::RunError::io(format!(
            "create repository iteration provenance directory: {error}"
        ))
    })?;
    std::fs::write(run_root.join(relative), &payload_bytes).map_err(|error| {
        crate::run_error::RunError::io(format!("write repository iteration provenance: {error}"))
    })?;
    let reference = serde_json::json!({
        "schema":"code-intel-artifact-ref.v1",
        "artifactSchema":crate::artifact_ref::REPOSITORY_ITERATION_SCHEMA,
        "type":crate::artifact_ref::REPOSITORY_ITERATION_TYPE,
        "path":relative,
        "sha256":crate::capability::sha256_hex(&payload_bytes),
        "consumedSnapshotIdentity":snapshot_identity,
    });
    nodes.insert(
        "repository.iteration".into(),
        serde_json::json!({
            "status":"succeeded",
            "verdict":"pass",
            "artifacts":[reference],
        }),
    );
    persist_manifest_handoff(run_root, manifest)
}

fn persist_manifest_handoff(
    run_root: &std::path::Path,
    manifest: &serde_json::Value,
) -> Result<(), crate::run_error::RunError> {
    let bytes = serde_json::to_vec(manifest).map_err(|error| {
        crate::run_error::RunError::contract(format!(
            "serialize authoritative terminal run manifest: {error}"
        ))
    })?;
    std::fs::write(run_root.join("run-manifest.json"), &bytes).map_err(|error| {
        crate::run_error::RunError::io(format!(
            "write authoritative terminal run manifest: {error}"
        ))
    })?;
    let reference = serde_json::json!({
        "schema":"code-intel-artifact-ref.v1",
        "artifactSchema":"code-intel-run-manifest.v1",
        "type":"run.manifest",
        "path":"run-manifest.json",
        "sha256":crate::capability::sha256_hex(&bytes),
        "consumedSnapshotIdentity":manifest["snapshotIdentity"],
    });
    std::fs::write(
        run_root.join("run-manifest-ref.json"),
        serde_json::to_vec(&reference).expect("run manifest Artifact Ref serializes"),
    )
    .map_err(|error| {
        crate::run_error::RunError::io(format!(
            "write authoritative run manifest Artifact Ref: {error}"
        ))
    })
}

pub(super) fn admit(
    outcome: crate::dag_coordinator::RunOutcome,
    publication: &super::execution_kernel::Publication,
) -> Result<(), crate::run_error::RunError> {
    crate::run_commit::validate_committed_run(&publication.path).map_err(map_commit_error)?;
    let artifact_root = publication
        .path
        .parent()
        .and_then(|repo_root| repo_root.parent())
        .ok_or_else(|| {
            crate::run_error::RunError::contract("committed publication has no artifact root")
        })?;
    let index = crate::artifact_index::rebuild(artifact_root).map_err(map_index_error)?;
    if outcome == crate::dag_coordinator::RunOutcome::Completed {
        require_completed_latest(&index, &publication.repo, &publication.name)?;
    }
    crate::artifact_index::write_index(&artifact_root.join("index.json"), &index)
        .map_err(map_index_error)
}

pub(super) fn require_completed_latest(
    index: &serde_json::Value,
    repo: &str,
    run: &str,
) -> Result<(), crate::run_error::RunError> {
    let indexed = index["entries"]
        .as_array()
        .and_then(|entries| entries.iter().find(|entry| entry["repo"] == repo))
        .and_then(|entry| entry["run"].as_str());
    if indexed == Some(run) {
        return Ok(());
    }
    Err(crate::run_error::RunError::contract(format!(
        "completed publication {repo}/{run} is not the latest completed-only index entry; selected {}",
        indexed.unwrap_or("<none>")
    )))
}

fn map_index_error(error: crate::artifact_index::IndexError) -> crate::run_error::RunError {
    match error {
        crate::artifact_index::IndexError::Contract(message) => {
            crate::run_error::RunError::contract(message)
        }
        crate::artifact_index::IndexError::HostIo(message) => {
            crate::run_error::RunError::io(message)
        }
    }
}

fn map_commit_error(error: crate::run_commit::CommitError) -> crate::run_error::RunError {
    match error {
        crate::run_commit::CommitError::Contract(message) => {
            crate::run_error::RunError::contract(message)
        }
        // Revalidating an already-committed handoff has no reachable collision
        // today, but folding it into the `Contract` arm is exactly the mistake
        // that made a taken publication name report as exit 65 on the kernel
        // side. Keep the classification honest on both mappers.
        crate::run_commit::CommitError::Collision(message) => {
            crate::run_error::RunError::publication_collision(message)
        }
        crate::run_commit::CommitError::HostIo(message) => crate::run_error::RunError::io(message),
        crate::run_commit::CommitError::Interrupted(phase) => {
            crate::run_error::RunError::new(75, format!("publication interrupted before {phase:?}"))
        }
    }
}
