use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

#[path = "../src/adapter_contract.rs"]
mod adapter_contract;
#[path = "../src/artifact_index.rs"]
mod artifact_index;
#[path = "../src/artifact_ref.rs"]
mod artifact_ref;
#[path = "../src/audit_report/mod.rs"]
mod audit_report;
#[path = "../src/capability.rs"]
mod capability;
#[path = "../src/capability_inventory.rs"]
mod capability_inventory;
#[path = "../src/run_commit.rs"]
mod run_commit;
#[path = "../src/snapshot.rs"]
mod snapshot;
#[path = "../src/stable_artifact.rs"]
mod stable_artifact;
#[path = "../src/staged_artifact.rs"]
mod staged_artifact;

use run_commit::CommitOptions;
use staged_artifact::{ArtifactWriteContract, StagedWriter};

const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SNAPSHOT_PAYLOAD: &[u8] = br#"{
  "schema":"code-intel-repository-snapshot.v1",
  "snapshot":{
    "identity":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "repoIdentity":"content-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "head":"fixture",
    "workingTreePolicy":"explicit_overlay",
    "scope":["."],
    "inputDigest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  },
  "dirtyOverlay":{
    "present":false,
    "digest":null,
    "paths":[],
    "members":{
      "trackedModified":[],
      "trackedDeleted":[],
      "untracked":[],
      "renamed":[],
      "typeChanged":[],
      "staged":[]
    },
    "ignoredPolicy":"excluded_by_git_ignore"
  },
  "repository":{"kind":"unversioned"}
}"#;
static SEQ: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);
impl Temp {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-a08-{label}-{}-{nonce}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn inventory_contract() -> ArtifactWriteContract {
    ArtifactWriteContract {
        artifact_schema: "code-intel-repository-snapshot.v1",
        artifact_type: "repository.snapshot",
        max_bytes: 1024,
        validate_payload: |bytes| {
            if bytes == SNAPSHOT_PAYLOAD {
                Ok(())
            } else {
                Err("invalid repository snapshot".into())
            }
        },
    }
}

fn manifest_contract() -> ArtifactWriteContract {
    ArtifactWriteContract {
        artifact_schema: "code-intel-run-manifest.v1",
        artifact_type: "run.manifest",
        max_bytes: 1024 * 1024,
        validate_payload: run_commit::validate_run_manifest_bytes,
    }
}

fn provenance_contract() -> ArtifactWriteContract {
    let contract = artifact_ref::registered_contract(&json!({
        "artifactSchema":artifact_ref::REPOSITORY_ITERATION_SCHEMA,
        "type":artifact_ref::REPOSITORY_ITERATION_TYPE,
    }))
    .unwrap();
    ArtifactWriteContract {
        artifact_schema: contract.artifact_schema,
        artifact_type: contract.artifact_type,
        max_bytes: contract.max_bytes,
        validate_payload: contract.validate_payload,
    }
}

fn provenance_payload(
    run_identity: &str,
    repository_key: &str,
    publication_name: &str,
    snapshot_identity: &str,
) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema":artifact_ref::REPOSITORY_ITERATION_SCHEMA,
        "purpose":artifact_ref::REPOSITORY_ITERATION_PURPOSE,
        "runIdentity":run_identity,
        "snapshotIdentity":snapshot_identity,
        "repositoryKey":repository_key,
        "publicationName":publication_name,
        "producer":{
            "component":artifact_ref::REPOSITORY_ITERATION_PRODUCER_COMPONENT,
            "contract":artifact_ref::REPOSITORY_ITERATION_PRODUCER_CONTRACT,
            "version":artifact_ref::REPOSITORY_ITERATION_PRODUCER_VERSION,
        }
    }))
    .unwrap()
}

fn staged_with_outcome(
    root: &Path,
    run_identity: &str,
    outcome: &str,
    publication_name: &str,
) -> (staged_artifact::StagedArtifactSet, Value) {
    let mut writer = StagedWriter::begin(root, SNAPSHOT).unwrap();
    let nodes = match outcome {
        "completed" => {
            let inventory = writer
                .stage(SNAPSHOT_PAYLOAD, inventory_contract())
                .unwrap()
                .to_artifact_ref_value();
            let repository_key = root.file_name().unwrap().to_str().unwrap();
            let provenance = writer
                .stage(
                    &provenance_payload(run_identity, repository_key, publication_name, SNAPSHOT),
                    provenance_contract(),
                )
                .unwrap()
                .to_artifact_ref_value();
            json!({
                "repo.snapshot":{"status":"succeeded","verdict":"pass","artifacts":[inventory]},
                "repository.iteration":{"status":"succeeded","verdict":"pass","artifacts":[provenance]}
            })
        }
        "domain_failed" => json!({
            "doctor":{"status":"domain_failed","verdict":"fail","diagnostic":"fixture domain failure","artifacts":[]}
        }),
        other => panic!("unsupported fixture outcome: {other}"),
    };
    let manifest = json!({
        "schema":"code-intel-run-manifest.v1",
        "runIdentity":run_identity,
        "snapshotIdentity":SNAPSHOT,
        "outcome":outcome,
        "nodes":nodes
    });
    let manifest_ref = writer
        .stage(&serde_json::to_vec(&manifest).unwrap(), manifest_contract())
        .unwrap()
        .to_artifact_ref_value();
    (writer.seal().unwrap(), manifest_ref)
}

fn staged(
    root: &Path,
    run_identity: &str,
    publication_name: &str,
) -> (staged_artifact::StagedArtifactSet, Value) {
    staged_with_outcome(root, run_identity, "completed", publication_name)
}

fn staged_snapshot_only(
    root: &Path,
    run_identity: &str,
) -> (staged_artifact::StagedArtifactSet, Value) {
    let mut writer = StagedWriter::begin(root, SNAPSHOT).unwrap();
    let inventory = writer
        .stage(SNAPSHOT_PAYLOAD, inventory_contract())
        .unwrap()
        .to_artifact_ref_value();
    let manifest = json!({
        "schema":"code-intel-run-manifest.v1",
        "runIdentity":run_identity,
        "snapshotIdentity":SNAPSHOT,
        "outcome":"completed",
        "nodes":{
            "repo.snapshot":{"status":"succeeded","verdict":"pass","artifacts":[inventory]}
        }
    });
    let manifest_ref = writer
        .stage(&serde_json::to_vec(&manifest).unwrap(), manifest_contract())
        .unwrap()
        .to_artifact_ref_value();
    (writer.seal().unwrap(), manifest_ref)
}

fn staged_custom_provenance(
    root: &Path,
    run_identity: &str,
    payloads: &[Vec<u8>],
    status: &str,
) -> (staged_artifact::StagedArtifactSet, Value) {
    let mut writer = StagedWriter::begin(root, SNAPSHOT).unwrap();
    let inventory = writer
        .stage(SNAPSHOT_PAYLOAD, inventory_contract())
        .unwrap()
        .to_artifact_ref_value();
    let provenance = payloads
        .iter()
        .map(|payload| {
            writer
                .stage(payload, provenance_contract())
                .unwrap()
                .to_artifact_ref_value()
        })
        .collect::<Vec<_>>();
    let provenance_node = match status {
        "succeeded" => json!({
            "status":"succeeded","verdict":"pass","artifacts":provenance
        }),
        "domain_failed" => json!({
            "status":"domain_failed","verdict":"fail",
            "diagnostic":"fixture wrong provenance status","artifacts":provenance
        }),
        other => panic!("unsupported provenance status: {other}"),
    };
    let manifest = json!({
        "schema":"code-intel-run-manifest.v1",
        "runIdentity":run_identity,
        "snapshotIdentity":SNAPSHOT,
        "outcome":"completed",
        "nodes":{
            "repo.snapshot":{"status":"succeeded","verdict":"pass","artifacts":[inventory]},
            "repository.iteration":provenance_node
        }
    });
    let manifest_ref = writer
        .stage(&serde_json::to_vec(&manifest).unwrap(), manifest_contract())
        .unwrap()
        .to_artifact_ref_value();
    (writer.seal().unwrap(), manifest_ref)
}

fn staged_non_repository_iteration(root: &Path) -> (staged_artifact::StagedArtifactSet, Value) {
    let mut writer = StagedWriter::begin(root, SNAPSHOT).unwrap();
    let manifest = json!({
        "schema":"code-intel-run-manifest.v1",
        "runIdentity":"dag-v1:dddd",
        "snapshotIdentity":SNAPSHOT,
        "outcome":"completed",
        "nodes":{
            "decision_record":{
                "status":"succeeded",
                "verdict":"pass",
                "artifacts":[]
            }
        }
    });
    let manifest_ref = writer
        .stage(&serde_json::to_vec(&manifest).unwrap(), manifest_contract())
        .unwrap()
        .to_artifact_ref_value();
    (writer.seal().unwrap(), manifest_ref)
}

#[test]
fn completed_governance_run_cannot_enter_the_repository_authority_index() {
    let tree = Temp::new("governance-purpose");
    let repo = tree.0.join("repo-a");
    fs::create_dir(&repo).unwrap();
    let (set, manifest_ref) = staged_non_repository_iteration(&repo);
    run_commit::commit(set, &manifest_ref, "decision-001", CommitOptions::default()).unwrap();

    let index = artifact_index::rebuild(&tree.0).unwrap();
    assert!(index["entries"].as_array().unwrap().is_empty());
    assert!(index["diagnostics"].as_array().unwrap().iter().any(|item| {
        item["repo"] == "repo-a"
            && item["run"] == "decision-001"
            && item["classification"] == "non_repository_iteration"
    }));
}

#[test]
fn snapshot_only_admin_commit_cannot_enter_the_repository_authority_index() {
    let tree = Temp::new("snapshot-only-admin");
    let source = tree.0.join("source");
    let repo = tree.0.join("repo-a");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&repo).unwrap();
    let (set, manifest_ref) = staged_snapshot_only(&source, "dag-v1:aabb");
    let manifest_ref_path = set.path().join("manifest-ref.json");
    fs::write(
        &manifest_ref_path,
        serde_json::to_vec(&manifest_ref).unwrap(),
    )
    .unwrap();
    let command = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .arg("run")
        .arg("commit")
        .arg("--source-root")
        .arg(set.path())
        .arg("--authority-root")
        .arg(&repo)
        .arg("--manifest-ref")
        .arg(manifest_ref_path)
        .arg("--final-name")
        .arg("run-001")
        .output()
        .unwrap();
    assert!(
        command.status.success(),
        "{}",
        String::from_utf8_lossy(&command.stderr)
    );

    let index = artifact_index::rebuild(&tree.0).unwrap();
    assert!(index["entries"].as_array().unwrap().is_empty());
    assert!(index["diagnostics"].as_array().unwrap().iter().any(|item| {
        item["repo"] == "repo-a"
            && item["run"] == "run-001"
            && item["classification"] == "non_repository_iteration"
    }));
}

#[test]
fn repository_iteration_provenance_requires_exact_authority_bindings() {
    let tree = Temp::new("provenance-bindings");
    let cases = [
        ("wrong-run", "dag-v1:ccdd", SNAPSHOT, "wrong-run", "run"),
        (
            "wrong-snapshot",
            "dag-v1:aabb",
            &"b".repeat(64),
            "wrong-snapshot",
            "run",
        ),
        ("wrong-repo", "dag-v1:aabb", SNAPSHOT, "another-repo", "run"),
        (
            "wrong-publication",
            "dag-v1:aabb",
            SNAPSHOT,
            "wrong-publication",
            "other-run",
        ),
    ];
    for (repo_name, payload_run, payload_snapshot, payload_repo, payload_publication) in cases {
        let repo = tree.0.join(repo_name);
        fs::create_dir(&repo).unwrap();
        let payload = provenance_payload(
            payload_run,
            payload_repo,
            payload_publication,
            payload_snapshot,
        );
        let (set, manifest_ref) =
            staged_custom_provenance(&repo, "dag-v1:aabb", &[payload], "succeeded");
        run_commit::commit(set, &manifest_ref, "run", CommitOptions::default()).unwrap();
    }

    let index = artifact_index::rebuild(&tree.0).unwrap();
    assert!(index["entries"].as_array().unwrap().is_empty());
    assert_eq!(
        index["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["classification"] == "non_repository_iteration")
            .count(),
        4
    );
}

#[test]
fn duplicate_or_wrong_status_provenance_is_not_repository_authority() {
    let tree = Temp::new("provenance-cardinality-status");
    for (repo_name, status, duplicate) in [
        ("duplicate", "succeeded", true),
        ("wrong-status", "domain_failed", false),
    ] {
        let repo = tree.0.join(repo_name);
        fs::create_dir(&repo).unwrap();
        let first = provenance_payload("dag-v1:aabb", repo_name, "run", SNAPSHOT);
        let mut payloads = vec![first];
        if duplicate {
            payloads.push(provenance_payload(
                "dag-v1:aabb",
                "another-repo",
                "run",
                SNAPSHOT,
            ));
        }
        let (set, manifest_ref) = staged_custom_provenance(&repo, "dag-v1:aabb", &payloads, status);
        run_commit::commit(set, &manifest_ref, "run", CommitOptions::default()).unwrap();
    }
    let index = artifact_index::rebuild(&tree.0).unwrap();
    assert!(index["entries"].as_array().unwrap().is_empty());
    assert_eq!(
        index["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["classification"] == "non_repository_iteration")
            .count(),
        2
    );
}

#[test]
fn repository_iteration_payload_contract_rejects_strict_field_mutations() {
    let contract = provenance_contract();
    let valid: Value = serde_json::from_slice(&provenance_payload(
        "dag-v1:aabb",
        "repo-a",
        "run-001",
        SNAPSHOT,
    ))
    .unwrap();
    let mut cases = Vec::new();
    for pointer in [
        "/schema",
        "/purpose",
        "/runIdentity",
        "/snapshotIdentity",
        "/repositoryKey",
        "/publicationName",
        "/producer/component",
        "/producer/contract",
        "/producer/version",
    ] {
        let mut mutated = valid.clone();
        *mutated.pointer_mut(pointer).unwrap() =
            if matches!(pointer, "/repositoryKey" | "/publicationName") {
                json!("../wrong")
            } else {
                json!("wrong")
            };
        cases.push(mutated);
    }
    let mut extra = valid.clone();
    extra["unexpected"] = json!(true);
    cases.push(extra);
    let mut producer_extra = valid.clone();
    producer_extra["producer"]["unexpected"] = json!(true);
    cases.push(producer_extra);
    for (index, invalid) in cases.into_iter().enumerate() {
        assert!(
            (contract.validate_payload)(&serde_json::to_vec(&invalid).unwrap()).is_err(),
            "strict provenance field case {index} was accepted"
        );
    }
    assert!(artifact_ref::registered_contract(&json!({
        "artifactSchema":artifact_ref::REPOSITORY_ITERATION_SCHEMA,
        "type":"wrong.type"
    }))
    .is_err());
    assert!(artifact_ref::registered_contract(&json!({
        "artifactSchema":"wrong.schema",
        "type":artifact_ref::REPOSITORY_ITERATION_TYPE
    }))
    .is_err());
}

#[test]
fn complete_and_staged_side_by_side_indexes_only_complete_run() {
    let tree = Temp::new("smallest");
    let repo = tree.0.join("repo-a");
    fs::create_dir(&repo).unwrap();
    let (set, manifest_ref) = staged(&repo, "dag-v1:aabb", "run-001");
    run_commit::commit(set, &manifest_ref, "run-001", CommitOptions::default()).unwrap();
    let (staged_set, _) = staged(&repo, "dag-v1:ccdd", "run-002");
    assert!(staged_set.path().is_dir());

    let result = artifact_index::rebuild(&tree.0).unwrap();

    assert_eq!(result["schema"], "code-intel-artifact-index.v1");
    assert_eq!(result["entries"].as_array().unwrap().len(), 1);
    assert_eq!(result["entries"][0]["repo"], "repo-a");
    assert_eq!(result["entries"][0]["run"], "run-001");
    assert_eq!(result["entries"][0]["runIdentity"], "dag-v1:aabb");
    assert!(result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["classification"] == "staging"));
}

fn committed_repo(root: &Path, repo_name: &str, run_name: &str, identity: &str) -> PathBuf {
    let repo = root.join(repo_name);
    fs::create_dir_all(&repo).unwrap();
    let (set, manifest_ref) = staged(&repo, identity, run_name);
    run_commit::commit(set, &manifest_ref, run_name, CommitOptions::default())
        .unwrap()
        .final_path
}

#[test]
fn forged_marker_manifest_and_artifact_bindings_are_diagnosed_and_rejected() {
    let tree = Temp::new("forged");
    let bad_digest = committed_repo(&tree.0, "bad-digest", "run", "dag-v1:aabb");
    let mut marker: Value =
        serde_json::from_slice(&fs::read(bad_digest.join("run-complete.json")).unwrap()).unwrap();
    marker["manifest"]["sha256"] = json!("0".repeat(64));
    fs::write(
        bad_digest.join("run-complete.json"),
        serde_json::to_vec(&marker).unwrap(),
    )
    .unwrap();

    let bad_identity = committed_repo(&tree.0, "bad-identity", "run", "dag-v1:aabb");
    let mut marker: Value =
        serde_json::from_slice(&fs::read(bad_identity.join("run-complete.json")).unwrap()).unwrap();
    marker["runIdentity"] = json!("dag-v1:ccdd");
    fs::write(
        bad_identity.join("run-complete.json"),
        serde_json::to_vec(&marker).unwrap(),
    )
    .unwrap();

    let bad_snapshot = committed_repo(&tree.0, "bad-snapshot", "run", "dag-v1:aabb");
    let mut marker: Value =
        serde_json::from_slice(&fs::read(bad_snapshot.join("run-complete.json")).unwrap()).unwrap();
    marker["snapshotIdentity"] = json!("b".repeat(64));
    fs::write(
        bad_snapshot.join("run-complete.json"),
        serde_json::to_vec(&marker).unwrap(),
    )
    .unwrap();

    let bad_artifact = committed_repo(&tree.0, "bad-artifact", "run", "dag-v1:aabb");
    let (_, manifest) = run_commit::validate_committed_run(&bad_artifact).unwrap();
    let artifact_path = manifest["nodes"]["repo.snapshot"]["artifacts"][0]["path"]
        .as_str()
        .unwrap();
    fs::write(bad_artifact.join(artifact_path), b"tampered evidence\n").unwrap();

    let bad_provenance = committed_repo(&tree.0, "bad-provenance", "run", "dag-v1:aabb");
    let (_, manifest) = run_commit::validate_committed_run(&bad_provenance).unwrap();
    let provenance_path = manifest["nodes"]["repository.iteration"]["artifacts"][0]["path"]
        .as_str()
        .unwrap();
    fs::write(
        bad_provenance.join(provenance_path),
        b"tampered provenance\n",
    )
    .unwrap();

    let result = artifact_index::rebuild(&tree.0).unwrap();
    assert_eq!(result["entries"], json!([]));
    let forged = result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["classification"] == "forged")
        .count();
    assert_eq!(forged, 5);
}

#[test]
fn incomplete_and_legacy_runs_are_diagnostic_only_and_newer_invalid_does_not_win() {
    let tree = Temp::new("diagnostics");
    let valid = committed_repo(&tree.0, "repo-a", "run-001", "dag-v1:aabb");
    assert!(valid.is_dir());
    let incomplete = committed_repo(&tree.0, "repo-a", "run-999", "dag-v1:ccdd");
    fs::remove_file(incomplete.join("run-complete.json")).unwrap();
    let legacy = tree.0.join("legacy-repo/20260713-120000");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("report.json"), b"{}\n").unwrap();

    let result = artifact_index::rebuild(&tree.0).unwrap();
    assert_eq!(result["entries"].as_array().unwrap().len(), 1);
    assert_eq!(result["entries"][0]["run"], "run-001");
    assert!(result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["run"] == "run-999" && item["classification"] == "incomplete"));
    assert!(result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["repo"] == "legacy-repo" && item["classification"] == "legacy"));
}

#[test]
fn newer_committed_non_completed_run_is_diagnostic_only_and_never_becomes_latest() {
    let tree = Temp::new("non-completed");
    committed_repo(&tree.0, "repo-a", "run-001", "dag-v1:aabb");
    let repo = tree.0.join("repo-a");
    let (set, manifest_ref) = staged_with_outcome(&repo, "dag-v1:ccdd", "domain_failed", "run-999");
    run_commit::commit(set, &manifest_ref, "run-999", CommitOptions::default()).unwrap();

    let result = artifact_index::rebuild(&tree.0).unwrap();

    assert_eq!(result["entries"].as_array().unwrap().len(), 1);
    assert_eq!(result["entries"][0]["run"], "run-001");
    assert_eq!(result["entries"][0]["outcome"], "completed");
    assert!(result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| {
            item["run"] == "run-999"
                && item["classification"] == "non_completed"
                && item["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("domain_failed"))
        }));
}

#[test]
fn rebuild_and_incremental_are_byte_equivalent_and_stably_sorted() {
    let tree = Temp::new("equivalence");
    committed_repo(&tree.0, "repo-z", "run-001", "dag-v1:aabb");
    committed_repo(&tree.0, "repo-a", "run-001", "dag-v1:ccdd");
    let first = artifact_index::rebuild(&tree.0).unwrap();
    let second = artifact_index::incremental(&tree.0, &first).unwrap();

    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    assert_eq!(first["entries"][0]["repo"], "repo-a");
    assert_eq!(first["entries"][1]["repo"], "repo-z");
    assert!(artifact_index::incremental(&tree.0, &json!({})).is_err());
}

#[test]
fn incremental_rejects_every_invalid_nested_existing_index_shape() {
    let tree = Temp::new("nested-existing");
    committed_repo(&tree.0, "repo-a", "run-001", "dag-v1:aabb");
    let valid = artifact_index::rebuild(&tree.0).unwrap();

    let mut cases = Vec::new();
    let mut value = valid.clone();
    value["entries"][0]["unexpected"] = json!(true);
    cases.push(value);
    let mut value = valid.clone();
    value["entries"][0]["repo"] = json!("../escape");
    cases.push(value);
    let mut value = valid.clone();
    value["entries"][0]["run"] = json!("");
    cases.push(value);
    let mut value = valid.clone();
    value["entries"][0]["runIdentity"] = json!("dag-v1:not-hex");
    cases.push(value);
    let mut value = valid.clone();
    value["entries"][0]["snapshotIdentity"] = json!(7);
    cases.push(value);
    let mut value = valid.clone();
    value["entries"][0]["outcome"] = json!("invented");
    cases.push(value);
    let mut value = valid.clone();
    value["entries"][0]["manifest"]["path"] = json!("../manifest.json");
    cases.push(value);
    let mut value = valid.clone();
    value["entries"][0]["artifactRefs"][0]["path"] = json!("../outside.bin");
    cases.push(value);

    let valid_diagnostic = json!({
        "repo": "repo-a",
        "run": "run-002",
        "classification": "incomplete",
        "reason": "A07 completion marker is absent"
    });
    let mut value = valid.clone();
    value["diagnostics"] = json!([valid_diagnostic.clone()]);
    value["diagnostics"][0]["unexpected"] = json!(true);
    cases.push(value);
    let mut value = valid.clone();
    value["diagnostics"] = json!([valid_diagnostic.clone()]);
    value["diagnostics"][0]["classification"] = json!("accepted-anyway");
    cases.push(value);
    let mut value = valid.clone();
    value["diagnostics"] = json!([valid_diagnostic]);
    value["diagnostics"][0]["run"] = json!("../escape");
    cases.push(value);

    for (index, invalid) in cases.into_iter().enumerate() {
        assert!(
            artifact_index::incremental(&tree.0, &invalid).is_err(),
            "invalid nested existing-index case {index} was accepted"
        );
    }
}

#[test]
fn write_index_replaces_an_existing_index_and_leaves_no_backup_behind() {
    let tree = Temp::new("rewrite");
    committed_repo(&tree.0, "repo-a", "run-001", "dag-v1:aabb");
    let output = tree.0.join("index.json");
    let index = artifact_index::rebuild(&tree.0).unwrap();
    artifact_index::write_index(&output, &index).unwrap();
    artifact_index::write_index(&output, &index).unwrap();

    let written: Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(written, index);
    assert!(
        !tree.0.join("index.json.bak").exists(),
        "publish must remove the transient .bak once the new index is in place"
    );
}

#[test]
fn production_cli_writes_the_registered_committed_only_schema() {
    let tree = Temp::new("cli");
    committed_repo(&tree.0, "repo-a", "run-001", "dag-v1:aabb");
    let output = tree.0.join("index.json");
    let command = Command::new(env!("CARGO_BIN_EXE_code-intel"))
        .args([
            "artifact",
            "index",
            "--artifact-root",
            tree.0.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--operation",
            "rebuild",
        ])
        .output()
        .unwrap();

    assert!(
        command.status.success(),
        "{}",
        String::from_utf8_lossy(&command.stderr)
    );
    let stdout: Value = serde_json::from_slice(&command.stdout).unwrap();
    let written: Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(stdout, written);
    assert_eq!(written["schema"], "code-intel-artifact-index.v1");

    let registry: Value = serde_json::from_slice(
        &fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../orchestration/integrations.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let route = registry["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "artifact.index-committed-only")
        .unwrap();
    assert_eq!(
        route["entrypoint"],
        "crates/code-intel-cli/src/artifact_index.rs"
    );
    assert!(route["commands"]["facade"]
        .as_str()
        .unwrap()
        .contains("legacy/update-code-intel-index.ps1"));
    assert!(route["artifactContract"].as_array().unwrap().iter().any(
        |contract| contract == "orchestration/schemas/code-intel-artifact-index.v1.schema.json"
    ));

    let schema: Value = serde_json::from_slice(
        &fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../orchestration/schemas/code-intel-artifact-index.v1.schema.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["properties"]["schema"]["const"],
        "code-intel-artifact-index.v1"
    );
}
