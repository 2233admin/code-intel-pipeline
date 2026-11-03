mod common;

use common::TempDir;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

#[path = "../src/adapter_contract.rs"]
mod adapter_contract;
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

use run_commit::{CommitOptions, PublicationPhase};
use staged_artifact::{ArtifactWriteContract, StagedWriter};

const SNAPSHOT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn inventory_contract() -> ArtifactWriteContract {
    ArtifactWriteContract {
        artifact_schema: "code-intel-file-inventory.v1",
        artifact_type: "inventory.files",
        max_bytes: 1024,
        validate_payload: |bytes| {
            if bytes == b"portable evidence\n" {
                Ok(())
            } else {
                Err("invalid inventory".into())
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

fn staged(root: &Path) -> (staged_artifact::StagedArtifactSet, Value) {
    let mut writer = StagedWriter::begin(root, SNAPSHOT).unwrap();
    let inventory = writer
        .stage(b"portable evidence\n", inventory_contract())
        .unwrap()
        .to_artifact_ref_value();
    let manifest = json!({
        "schema":"code-intel-run-manifest.v1",
        "runIdentity":"dag-v1:aabb",
        "snapshotIdentity":SNAPSHOT,
        "outcome":"completed",
        "nodes":{"inventory":{"status":"succeeded","verdict":"pass","artifacts":[inventory]}}
    });
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    let manifest_ref = writer
        .stage(&manifest_bytes, manifest_contract())
        .unwrap()
        .to_artifact_ref_value();
    (writer.seal().unwrap(), manifest_ref)
}

#[test]
fn staged_run_is_promoted_and_completion_marker_is_published_last() {
    let tree = TempDir::new("complete");
    let (set, manifest_ref) = staged(tree.path());
    let result =
        run_commit::commit(set, &manifest_ref, "run-001", CommitOptions::default()).unwrap();
    assert_eq!(result.final_path, tree.path().join("run-001"));
    assert_eq!(result.marker["runIdentity"], "dag-v1:aabb");
    assert_eq!(result.marker["snapshotIdentity"], SNAPSHOT);
    assert_eq!(result.marker["manifest"]["sha256"], manifest_ref["sha256"]);
    assert!(result.final_path.join("run-complete.json").is_file());
    assert_eq!(run_commit::classify(&result.final_path), "committed");
    assert!(!tree
        .path()
        .join("run-001")
        .join("artifact-index.json")
        .exists());
}

#[test]
fn every_publication_phase_is_fail_closed_and_post_promotion_is_recoverable() {
    for phase in [
        PublicationPhase::Prevalidate,
        PublicationPhase::Rename,
        PublicationPhase::DirectorySync,
        PublicationPhase::MarkerTemp,
        PublicationPhase::MarkerPublish,
        PublicationPhase::PostMarkerVerify,
        PublicationPhase::Rollback,
    ] {
        let tree = TempDir::new(&format!("phase-{phase:?}"));
        let (set, manifest_ref) = staged(tree.path());
        let error = run_commit::commit(
            set,
            &manifest_ref,
            "run",
            CommitOptions {
                interrupt_before: Some(phase),
                ..CommitOptions::default()
            },
        )
        .unwrap_err();
        assert!(matches!(error, run_commit::CommitError::Interrupted(value) if value == phase));
        let final_path = tree.path().join("run");
        assert!(!final_path.join("run-complete.json").exists(), "{phase:?}");
        if phase == PublicationPhase::MarkerPublish {
            assert!(
                fs::read_dir(&final_path).unwrap().all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".run-complete.json.tmp.")),
                "owned marker temp was not cleaned"
            );
        }
        if matches!(
            phase,
            PublicationPhase::Prevalidate | PublicationPhase::Rename
        ) {
            assert!(!final_path.exists(), "{phase:?}");
        } else {
            assert_eq!(
                run_commit::classify(&final_path),
                "legacy-uncommitted",
                "{phase:?}"
            );
            let recovered =
                run_commit::recover(&final_path, &manifest_ref, CommitOptions::default()).unwrap();
            assert_eq!(run_commit::classify(&recovered.final_path), "committed");
        }
    }
}

#[test]
fn prevalidation_rechecks_digest_schema_snapshot_and_manifest_completeness() {
    let tree = TempDir::new("tamper");
    let (set, manifest_ref) = staged(tree.path());
    fs::write(
        set.path().join(manifest_ref["path"].as_str().unwrap()),
        b"{}\n",
    )
    .unwrap();
    assert!(run_commit::commit(set, &manifest_ref, "run", CommitOptions::default()).is_err());
    assert!(!tree.path().join("run").exists());

    let invalid = json!({"schema":"code-intel-run-manifest.v1","runIdentity":"dag-v1:aabb","snapshotIdentity":SNAPSHOT,"outcome":"incomplete","nodes":{"x":{"status":"running"}}});
    assert!(
        run_commit::validate_run_manifest_bytes(&serde_json::to_vec(&invalid).unwrap()).is_err()
    );
}

#[test]
fn competing_destination_and_marker_are_preserved() {
    let tree = TempDir::new("competitor-dir");
    let competitor = tree.path().join("run");
    fs::create_dir(&competitor).unwrap();
    fs::write(competitor.join("sentinel.txt"), b"competitor").unwrap();
    let (set, manifest_ref) = staged(tree.path());
    assert!(run_commit::commit(set, &manifest_ref, "run", CommitOptions::default()).is_err());
    assert_eq!(
        fs::read(competitor.join("sentinel.txt")).unwrap(),
        b"competitor"
    );

    let tree = TempDir::new("competitor-marker");
    let (set, manifest_ref) = staged(tree.path());
    let _ = run_commit::commit(
        set,
        &manifest_ref,
        "run",
        CommitOptions {
            interrupt_before: Some(PublicationPhase::MarkerPublish),
            ..CommitOptions::default()
        },
    );
    let marker = tree.path().join("run/run-complete.json");
    fs::write(&marker, b"competitor-marker").unwrap();
    assert!(run_commit::recover(
        &tree.path().join("run"),
        &manifest_ref,
        CommitOptions::default()
    )
    .is_err());
    assert_eq!(fs::read(marker).unwrap(), b"competitor-marker");
}

#[test]
fn post_publish_sync_and_read_failures_roll_back_owned_marker() {
    for read_failure in [false, true] {
        let tree = TempDir::new(if read_failure {
            "read-failure"
        } else {
            "sync-failure"
        });
        let (set, manifest_ref) = staged(tree.path());
        let result = run_commit::commit(
            set,
            &manifest_ref,
            "run",
            CommitOptions {
                fail_marker_sync: !read_failure,
                fail_marker_read: read_failure,
                ..CommitOptions::default()
            },
        );
        assert!(result.is_err());
        let final_path = tree.path().join("run");
        assert!(!final_path.join("run-complete.json").exists());
        assert_eq!(run_commit::classify(&final_path), "legacy-uncommitted");
    }
}

#[test]
fn legacy_timestamp_run_is_readable_but_uncommitted() {
    let tree = TempDir::new("legacy");
    let legacy = tree.path().join("20260713-120000");
    fs::create_dir(&legacy).unwrap();
    fs::write(legacy.join("report.json"), b"{}\n").unwrap();
    assert_eq!(run_commit::classify(&legacy), "legacy-uncommitted");
    assert_eq!(fs::read(legacy.join("report.json")).unwrap(), b"{}\n");
}

#[test]
fn production_run_commit_cli_restages_a09_refs_through_a06_and_publishes() {
    let tree = TempDir::new("cli");
    let source_authority = tree.path().join("source-authority");
    let publication_authority = tree.path().join("publication-authority");
    fs::create_dir(&source_authority).unwrap();
    fs::create_dir(&publication_authority).unwrap();
    let (set, manifest_ref) = staged(&source_authority);
    let source =
        run_commit::commit(set, &manifest_ref, "source", CommitOptions::default()).unwrap();
    let manifest_ref_path = tree.path().join("manifest-ref.json");
    fs::write(
        &manifest_ref_path,
        serde_json::to_vec(&manifest_ref).unwrap(),
    )
    .unwrap();
    let raw = vec![
        "commit".to_string(),
        "--source-root".to_string(),
        source.final_path.display().to_string(),
        "--authority-root".to_string(),
        publication_authority.display().to_string(),
        "--manifest-ref".to_string(),
        manifest_ref_path.display().to_string(),
        "--final-name".to_string(),
        "published".to_string(),
    ];
    assert_eq!(run_commit::run_raw(&raw), 0);
    assert_eq!(
        run_commit::classify(&publication_authority.join("published")),
        "committed"
    );
}

#[test]
fn cli_publication_preserves_the_callers_manifest_bytes_and_digest() {
    let tree = TempDir::new("cli-verbatim");
    let source_root = tree.path().join("source");
    let publication_authority = tree.path().join("publication-authority");
    let objects = source_root.join("objects").join("sha256");
    fs::create_dir_all(&objects).unwrap();
    fs::create_dir(&publication_authority).unwrap();
    let inventory = b"portable evidence\n";
    let inventory_digest = capability::sha256_hex(inventory);
    fs::write(objects.join(&inventory_digest), inventory).unwrap();
    // Key order mirrors the original producer; serde re-serialization would
    // reorder it, so byte preservation is observable through the digest.
    let manifest_text = format!(
        concat!(
            "{{\"schema\":\"code-intel-run-manifest.v1\",\"runIdentity\":\"dag-v1:aabb\",",
            "\"snapshotIdentity\":\"{snap}\",\"outcome\":\"completed\",\"nodes\":{{\"inventory\":",
            "{{\"status\":\"succeeded\",\"verdict\":\"pass\",\"artifacts\":[{{",
            "\"schema\":\"code-intel-artifact-ref.v1\",",
            "\"artifactSchema\":\"code-intel-file-inventory.v1\",\"type\":\"inventory.files\",",
            "\"path\":\"objects/sha256/{inv}\",\"sha256\":\"{inv}\",",
            "\"consumedSnapshotIdentity\":\"{snap}\"}}]}}}}}}"
        ),
        snap = SNAPSHOT,
        inv = inventory_digest
    );
    let reparsed: Value = serde_json::from_str(&manifest_text).unwrap();
    assert_ne!(
        serde_json::to_vec(&reparsed).unwrap(),
        manifest_text.as_bytes(),
        "fixture must be non-canonical for byte preservation to be observable"
    );
    let manifest_digest = capability::sha256_hex(manifest_text.as_bytes());
    fs::write(objects.join(&manifest_digest), manifest_text.as_bytes()).unwrap();
    let manifest_ref = json!({
        "schema":"code-intel-artifact-ref.v1",
        "artifactSchema":"code-intel-run-manifest.v1",
        "type":"run.manifest",
        "path":format!("objects/sha256/{manifest_digest}"),
        "sha256":manifest_digest,
        "consumedSnapshotIdentity":SNAPSHOT
    });
    let manifest_ref_path = tree.path().join("manifest-ref.json");
    fs::write(
        &manifest_ref_path,
        serde_json::to_vec(&manifest_ref).unwrap(),
    )
    .unwrap();
    let raw = vec![
        "commit".to_string(),
        "--source-root".to_string(),
        source_root.display().to_string(),
        "--authority-root".to_string(),
        publication_authority.display().to_string(),
        "--manifest-ref".to_string(),
        manifest_ref_path.display().to_string(),
        "--final-name".to_string(),
        "published".to_string(),
    ];
    assert_eq!(run_commit::run_raw(&raw), 0);
    let published = publication_authority.join("published");
    let marker: Value =
        serde_json::from_slice(&fs::read(published.join("run-complete.json")).unwrap()).unwrap();
    assert_eq!(marker["manifest"]["sha256"], json!(manifest_digest));
    let committed = fs::read(
        published
            .join("objects")
            .join("sha256")
            .join(&manifest_digest),
    )
    .unwrap();
    assert_eq!(committed, manifest_text.as_bytes());
}
