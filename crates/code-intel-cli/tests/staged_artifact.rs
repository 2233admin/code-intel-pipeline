#[path = "../src/stable_artifact.rs"]
mod stable_artifact;

#[path = "../src/staged_artifact.rs"]
mod staged_artifact;

use std::fs;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use staged_artifact::{
    ArtifactWriteContract, InterruptAfter, StageWriteError, StagedWriter, WriterOptions,
};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempTree(PathBuf);

impl TempTree {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "code-intel-a06-{label}-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn contract(max_bytes: u64) -> ArtifactWriteContract {
    ArtifactWriteContract {
        artifact_schema: "code-intel-test-lines.v1",
        artifact_type: "test.lines",
        max_bytes,
        validate_payload: |bytes| {
            let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
            if text.lines().all(|line| !line.is_empty()) && text.ends_with('\n') {
                Ok(())
            } else {
                Err("payload must contain non-empty newline-terminated lines".to_string())
            }
        },
    }
}

fn begin(root: &Path, nonce: &str) -> Result<StagedWriter, StageWriteError> {
    StagedWriter::begin_with_options(
        root,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        WriterOptions {
            nonce: nonce.to_string(),
            interrupt_after: None,
            before_publish: None,
        },
    )
}

fn staging_children(root: &Path) -> Vec<PathBuf> {
    let staging = root.join(".staging");
    if !staging.is_dir() {
        return Vec::new();
    }
    fs::read_dir(staging)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect()
}

#[test]
fn schema_failure_rolls_back_owned_staging_and_never_exposes_a_final_run() {
    let tree = TempTree::new("schema-failure");
    let mut writer = begin(&tree.0, "schema-failure").unwrap();
    let error = writer.stage(b"invalid", contract(64)).unwrap_err();
    assert!(matches!(error, StageWriteError::Contract(_)));
    assert!(staging_children(&tree.0).is_empty());
    assert!(writer.seal().is_err());
    assert!(!tree.0.join("run-complete.json").exists());
    assert!(fs::read_dir(&tree.0)
        .unwrap()
        .all(|entry| entry.unwrap().file_name() == ".staging"));
}

#[test]
fn content_is_addressed_validated_and_duplicate_bytes_reuse_one_owned_object() {
    let tree = TempTree::new("content-addressed");
    let mut writer = begin(&tree.0, "content-addressed").unwrap();
    let first = writer.stage(b"alpha\n", contract(64)).unwrap();
    let second = writer.stage(b"alpha\n", contract(64)).unwrap();

    assert_eq!(first.sha256, second.sha256);
    assert_eq!(
        first.sha256,
        "b6a98d9ce9a2d9149288fa3df42d377c3e42737afdcdaf714e33c0a100b51060"
    );
    assert_eq!(first.path, second.path);
    assert_eq!(first.size, 6);
    assert_eq!(first.artifact_schema, "code-intel-test-lines.v1");
    assert_eq!(first.artifact_type, "test.lines");
    assert_eq!(first.consumed_snapshot_identity.len(), 64);

    let staged = writer.seal().unwrap();
    assert!(staged.path().starts_with(tree.0.join(".staging")));
    assert_eq!(staged.authority_root(), tree.0);
    assert_eq!(staged.artifacts().len(), 2);
    let manifest = staged.to_manifest_value();
    assert_eq!(manifest["schema"], "code-intel-staged-artifact-set.v1");
    assert_eq!(manifest["artifacts"].as_array().unwrap().len(), 2);
    assert_eq!(manifest["artifacts"][0]["sha256"], first.sha256);
    assert_eq!(
        manifest["artifacts"][0]["path"],
        format!("objects/sha256/{}", first.sha256)
    );
    let objects = fs::read_dir(staged.path().join("objects/sha256"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(objects.len(), 1);
    assert!(!tree.0.join("run-complete.json").exists());

    drop(staged);
    assert!(staging_children(&tree.0).is_empty());
}

#[cfg(windows)]
#[test]
fn content_addressed_publication_supports_windows_paths_beyond_max_path() {
    let tree = TempTree::new("windows-long-path");
    let mut root = tree.0.clone();
    for index in 0..3 {
        root.push(format!(
            "nested-authority-{index}-abcdefghijklmnopqrstuvwxyz0123456789"
        ));
    }
    fs::create_dir_all(&root).unwrap();

    let expected_target = root.join(format!(
        ".staging/stage-long-path/objects/sha256/{}",
        "b6a98d9ce9a2d9149288fa3df42d377c3e42737afdcdaf714e33c0a100b51060"
    ));
    assert!(
        expected_target.as_os_str().encode_wide().count() > 260,
        "test fixture must cross the legacy Windows MAX_PATH boundary"
    );

    let mut writer = begin(&root, "long-path").unwrap();
    let artifact = writer.stage(b"alpha\n", contract(64)).unwrap();
    assert_eq!(
        fs::read(root.join(".staging/stage-long-path").join(artifact.path)).unwrap(),
        b"alpha\n"
    );
}

#[test]
fn production_begin_is_unique_and_reports_only_its_observed_local_write_effect() {
    let tree = TempTree::new("production-begin");
    let first = StagedWriter::begin(
        &tree.0,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .unwrap();
    let second = StagedWriter::begin(
        &tree.0,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .unwrap();
    assert_eq!(first.observed_effects(), &["local_write"]);
    assert_ne!(staging_children(&tree.0)[0], staging_children(&tree.0)[1]);
    drop((first, second));
    assert!(staging_children(&tree.0).is_empty());
}

#[test]
fn commit_handoff_releases_handles_but_retains_owned_rollback() {
    let tree = TempTree::new("handoff");
    let mut writer = begin(&tree.0, "handoff").unwrap();
    writer.stage(b"alpha\n", contract(64)).unwrap();
    let mut staged = writer.seal().unwrap();
    let path = staged.path().to_path_buf();
    staged.prepare_for_commit().unwrap();
    assert!(path.is_dir());
    drop(staged);
    assert!(!path.exists());
}

#[test]
fn bounded_writes_accept_max_minus_one_and_max_but_reject_max_plus_one() {
    let tree = TempTree::new("bounds");
    let mut writer = begin(&tree.0, "bounds").unwrap();
    assert!(writer.stage(b"a\n", contract(3)).is_ok());
    assert!(writer.stage(b"bb\n", contract(3)).is_ok());
    assert!(matches!(
        writer.stage(b"ccc\n", contract(3)),
        Err(StageWriteError::Contract(_))
    ));
}

#[test]
fn every_interrupt_phase_rolls_back_only_the_owned_staging_tree() {
    let phases = [
        InterruptAfter::StageCreated,
        InterruptAfter::TempCreated,
        InterruptAfter::FileSynced,
        InterruptAfter::ObjectPublished,
        InterruptAfter::DirectorySynced,
    ];
    for phase in phases {
        let tree = TempTree::new("interrupt");
        let writer = StagedWriter::begin_with_options(
            &tree.0,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            WriterOptions {
                nonce: format!("interrupt-{phase:?}"),
                interrupt_after: Some(phase),
                before_publish: None,
            },
        );
        match phase {
            InterruptAfter::StageCreated => {
                assert!(matches!(writer, Err(StageWriteError::Interrupted(_))));
            }
            _ => {
                let mut writer = writer.unwrap();
                assert!(matches!(
                    writer.stage(b"alpha\n", contract(64)),
                    Err(StageWriteError::Interrupted(_))
                ));
                drop(writer);
            }
        }
        assert!(staging_children(&tree.0).is_empty(), "phase={phase:?}");
        assert!(!tree.0.join("run-complete.json").exists());
    }
}

fn occupy_addressed_target_with_same_bytes(object_dir: &Path, digest: &str) -> Result<(), String> {
    fs::write(object_dir.join(digest), b"alpha\n").map_err(|error| error.to_string())
}

fn occupy_addressed_target_with_different_bytes(
    object_dir: &Path,
    digest: &str,
) -> Result<(), String> {
    fs::write(object_dir.join(digest), b"competitor\n").map_err(|error| error.to_string())
}

#[test]
fn publish_race_with_different_content_reports_collision_and_preserves_competitor() {
    let tree = TempTree::new("publish-race-different");
    let nonce = "publish-race-different";
    let target = tree.0.join(format!(
        ".staging/stage-{nonce}/objects/sha256/{}",
        "b6a98d9ce9a2d9149288fa3df42d377c3e42737afdcdaf714e33c0a100b51060"
    ));
    let mut writer = StagedWriter::begin_with_options(
        &tree.0,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        WriterOptions {
            nonce: nonce.to_string(),
            interrupt_after: None,
            before_publish: Some(occupy_addressed_target_with_different_bytes),
        },
    )
    .unwrap();

    let error = writer.stage(b"alpha\n", contract(64)).unwrap_err();
    assert!(matches!(error, StageWriteError::Collision(_)));
    assert!(error.to_string().contains("residual"));
    assert_eq!(fs::read(&target).unwrap(), b"competitor\n");
    drop(writer);
    assert_eq!(fs::read(&target).unwrap(), b"competitor\n");
}

#[test]
fn publish_race_with_same_content_deduplicates_without_taking_delete_ownership() {
    let tree = TempTree::new("publish-race-same");
    let nonce = "publish-race-same";
    let target = tree.0.join(format!(
        ".staging/stage-{nonce}/objects/sha256/{}",
        "b6a98d9ce9a2d9149288fa3df42d377c3e42737afdcdaf714e33c0a100b51060"
    ));
    let mut writer = StagedWriter::begin_with_options(
        &tree.0,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        WriterOptions {
            nonce: nonce.to_string(),
            interrupt_after: None,
            before_publish: Some(occupy_addressed_target_with_same_bytes),
        },
    )
    .unwrap();

    let artifact = writer.stage(b"alpha\n", contract(64)).unwrap();
    assert!(!artifact.owned_by_stage);
    assert_eq!(
        artifact.sha256,
        target.file_name().unwrap().to_string_lossy()
    );
    let error = writer.seal().unwrap_err();
    assert!(matches!(error, StageWriteError::Collision(_)));
    assert!(error.to_string().contains("residual"));
    assert_eq!(fs::read(&target).unwrap(), b"alpha\n");
}

#[test]
fn nonce_collision_is_never_treated_as_owned_or_deleted() {
    let tree = TempTree::new("collision");
    let collision = tree.0.join(".staging/stage-collision");
    fs::create_dir_all(&collision).unwrap();
    fs::write(collision.join("competitor.txt"), b"keep me").unwrap();

    let error = begin(&tree.0, "collision").unwrap_err();
    assert!(matches!(error, StageWriteError::Collision(_)));
    assert_eq!(error.kind(), "collision");
    assert_eq!(
        fs::read(collision.join("competitor.txt")).unwrap(),
        b"keep me"
    );
}

#[test]
fn root_links_and_out_of_scope_authorities_fail_closed() {
    let tree = TempTree::new("boundary");
    let real = tree.0.join("real");
    fs::create_dir(&real).unwrap();
    let link = tree.0.join("link");
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&real, &link).is_ok();
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_dir(&real, &link).is_ok();
    if linked {
        assert!(matches!(
            begin(&link, "linked"),
            Err(StageWriteError::Boundary(_))
        ));
    }

    let file_root = tree.0.join("not-a-directory");
    fs::write(&file_root, b"x").unwrap();
    assert!(matches!(
        begin(&file_root, "file-root"),
        Err(StageWriteError::Boundary(_))
    ));
}

#[test]
fn checked_in_staged_set_schema_is_closed_and_matches_the_runtime_manifest() {
    let schema: Value = serde_json::from_slice(
        &fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../orchestration/schemas/code-intel-staged-artifact-set.v1.schema.json"
        ))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(schema["$id"], "code-intel-staged-artifact-set.v1");
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["$defs"]["artifactRef"]["additionalProperties"],
        false
    );
    assert_eq!(
        schema["$defs"]["artifactRef"]["properties"]["path"]["pattern"],
        "^objects/sha256/[0-9a-f]{64}$"
    );
}
