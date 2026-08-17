use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs, process};

use super::*;
use crate::change_impact;
use crate::evidence_query::QueryError;

#[test]
fn run_modes_map_to_the_existing_profiles() {
    assert_eq!(RunMode::parse("lite").unwrap(), RunMode::Lite);
    assert_eq!(RunMode::parse("normal").unwrap(), RunMode::Normal);
    assert_eq!(RunMode::parse("full").unwrap(), RunMode::Full);
    assert_eq!(RunMode::parse("fast").unwrap_err().exit_code(), 64);
}

#[test]
fn evidence_limit_is_one_closed_bound() {
    assert!(EvidenceQuery::new(None, None, None, Some(1)).is_ok());
    assert!(EvidenceQuery::new(None, None, None, Some(MAX_LIMIT)).is_ok());
    for invalid in [0, MAX_LIMIT + 1] {
        assert_eq!(
            EvidenceQuery::new(None, None, None, Some(invalid))
                .unwrap_err()
                .exit_code(),
            64
        );
    }
}

#[test]
fn query_and_impact_host_io_share_the_project_error_contract() {
    for error in [
        ProjectError::from(QueryError::HostIo("query host fault".into())),
        ProjectError::from(change_impact::ImpactError::HostIo(
            "impact host fault".into(),
        )),
    ] {
        assert_eq!(error.kind(), "host_io");
        assert_eq!(error.exit_code(), 74);
    }
}

#[test]
fn resolves_native_paths_and_repository_key_when_directories_contain_spaces() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = env::temp_dir().join(format!(
        "code intel project context spaces {} {nonce}",
        process::id()
    ));
    let repo = root.join("repository with spaces");
    let artifact_root = root.join("artifacts with spaces");
    fs::create_dir_all(&repo).expect("create repository fixture");
    fs::create_dir_all(&artifact_root).expect("create artifact fixture");

    let context = ProjectContext::resolve(
        ProjectSelector::new(repo.clone()).with_artifact_root(Some(artifact_root.clone())),
    )
    .expect("resolve project context");

    assert_eq!(context.repo_path(), fs::canonicalize(&repo).unwrap());
    assert_eq!(context.repo(), "repository with spaces");
    assert_eq!(context.artifact_root(), artifact_root);

    let _ = fs::remove_dir_all(root);
}

/// Issue #279: this call site and `execution_kernel.rs::nest_authority_root`
/// share the exact "create repository authority root: <raw OS error>"
/// message template — this one is what the `code-intel .` quick start
/// actually goes through. A file sitting where the per-repo authority
/// directory needs to go is a portable, deterministic way to exercise a
/// *genuine* block. (The Windows-only cross-volume-symlink false positive
/// this bug was actually filed for needs a real second drive to reproduce;
/// it is covered by `artifacts_tests.rs`'s `ensure_directory_*` tests
/// instead, against the shared helper directly.)
#[test]
fn run_names_the_authority_root_path_when_something_real_blocks_it() {
    let root = env::temp_dir().join(format!(
        "code-intel-project-context-authority-blocked-{}",
        process::id()
    ));
    let repo = root.join("fixture-repo");
    let artifact_root = root.join("artifacts");
    fs::create_dir_all(&repo).expect("create repository fixture");
    fs::create_dir_all(&artifact_root).expect("create artifact root fixture");
    let blocked = artifact_root.join("fixture-repo");
    fs::write(&blocked, b"not a directory").expect("fixture blocking file");

    let context = ProjectContext::resolve(
        ProjectSelector::new(repo.clone()).with_artifact_root(Some(artifact_root.clone())),
    )
    .expect("resolve project context");

    // `RunAnswer` does not derive `Debug`, so `expect_err` (which requires it
    // on the `Ok` side too) is not available here — match instead.
    match context.run(RunIntent::new(RunMode::Lite)) {
        Ok(_) => panic!("a real file must still block authority root creation"),
        Err(error) => {
            assert_eq!(error.exit_code(), 74);
            assert!(
                error.message().contains(&blocked.display().to_string()),
                "message omits the blocked path: {}",
                error.message()
            );
        }
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn explicit_repository_key_cannot_escape_the_artifact_root() {
    let root = env::temp_dir().join(format!("code-intel-project-context-key-{}", process::id()));
    fs::create_dir_all(&root).expect("create repository fixture");

    for key in ["", " ", ".", "..", "nested/repo", "nested\\repo"] {
        let error = ProjectContext::resolve(
            ProjectSelector::new(root.clone()).with_repo(Some(key.to_string())),
        )
        .unwrap_err();
        assert_eq!(error.exit_code(), 64, "{key:?}");
    }

    let _ = fs::remove_dir_all(root);
}
