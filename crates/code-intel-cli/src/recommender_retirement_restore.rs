//! Rust port of `Restore-RecommenderLegacyBranch.ps1`: proves the deletion
//! diff computed by `recommender_retirement_diff` is reversible, by
//! recovering the retired inline recommender from git history into a
//! disposable rehearsal copy (never the live file, by default). See
//! `recommender_retirement_packet`'s module doc for the overall port's
//! scope.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::json;
use serde_json::Value;

use crate::recommender_retirement_shared::{
    find_bounded_block, BRANCH_ID, CURRENT_FUNCTIONS_START, CURRENT_INVOCATION_START,
    FUNCTIONS_END, INVOCATION_END, LEGACY_FUNCTIONS_START, LEGACY_INVOCATION_START,
};

pub(crate) enum RestoreMode {
    /// Extract into a fresh, exclusive rehearsal directory (never touches a
    /// real checkout).
    Rehearsal(PathBuf),
    /// Apply directly to an existing target path -- an explicit, bounded,
    /// independently-authorized action, never the default.
    Apply(PathBuf),
}

/// Restore the historical inline recommender branch from `source_revision`
/// into a rehearsal copy (or, in `Apply` mode, a real target) of
/// `run-code-intel.ps1`. Proves the deletion diff is reversible without
/// ever writing to the live file by default.
pub(crate) fn restore_legacy_branch(
    repo_root: &Path,
    mode: RestoreMode,
    source_revision: &str,
) -> Result<Value, String> {
    let run_path = repo_root.join("run-code-intel.ps1");
    if !run_path.is_file() {
        return Err(format!(
            "run-code-intel.ps1 is missing from repository root: {}",
            repo_root.display()
        ));
    }

    let (target_path, rehearsal) = match mode {
        RestoreMode::Rehearsal(rehearsal_root) => {
            if rehearsal_root.exists() {
                return Err(format!(
                    "rollback rehearsal root must not already exist: {}",
                    rehearsal_root.display()
                ));
            }
            fs::create_dir_all(&rehearsal_root).map_err(|e| e.to_string())?;
            let target = rehearsal_root.join("run-code-intel.ps1");
            fs::copy(&run_path, &target).map_err(|e| e.to_string())?;
            (target, true)
        }
        RestoreMode::Apply(target) => (target, false),
    };
    if !target_path.is_file() {
        return Err(format!(
            "rollback target is missing: {}",
            target_path.display()
        ));
    }

    // `git show <rev>:<path>` is always repository-root-relative; the
    // archive move relocated run-code-intel.ps1, so try both locations
    // rather than pinning either.
    let legacy_candidates = ["legacy/run-code-intel.ps1", "run-code-intel.ps1"];
    let mut legacy_text = None;
    for candidate in legacy_candidates {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .arg("show")
            .arg(format!("{source_revision}:{candidate}"))
            .output();
        if let Ok(output) = output {
            if output.status.success() && !output.stdout.is_empty() {
                legacy_text = Some(String::from_utf8_lossy(&output.stdout).into_owned());
                break;
            }
        }
    }
    let legacy = legacy_text.ok_or_else(|| {
        format!(
            "cannot load legacy recommender source from {source_revision} at any of: {}",
            legacy_candidates.join(", ")
        )
    })?;
    let current = fs::read_to_string(&target_path).map_err(|e| e.to_string())?;

    let (lf_s, lf_e) =
        find_bounded_block(&legacy, LEGACY_FUNCTIONS_START, FUNCTIONS_END).map_err(|_| {
            format!(
                "legacy recommender markers are absent from {source_revision}:run-code-intel.ps1"
            )
        })?;
    let (li_s, li_e) = find_bounded_block(&legacy, LEGACY_INVOCATION_START, INVOCATION_END)
        .map_err(|_| {
            format!(
                "legacy recommender markers are absent from {source_revision}:run-code-intel.ps1"
            )
        })?;
    let legacy_functions = legacy[lf_s..lf_e].to_string();
    let legacy_invocation = legacy[li_s..li_e].to_string();

    let (cf_s, cf_e) = find_bounded_block(&current, CURRENT_FUNCTIONS_START, FUNCTIONS_END)
        .map_err(|_| {
            "target does not contain the retired recommender adapter markers".to_string()
        })?;
    let mut restored = format!(
        "{}{}{}",
        &current[..cf_s],
        legacy_functions,
        &current[cf_e..]
    );
    let (ci_s, ci_e) = find_bounded_block(&restored, CURRENT_INVOCATION_START, INVOCATION_END)
        .map_err(|_| {
            "target does not contain the retired recommender adapter markers".to_string()
        })?;
    restored = format!(
        "{}{}{}",
        &restored[..ci_s],
        legacy_invocation,
        &restored[ci_e..]
    );

    if !restored.contains("function Invoke-WorkflowStackDetector")
        || !restored
            .contains("Invoke-WorkflowStackDetector -RepoPath $repoPath -AutoMode $AutoOpenSpec")
    {
        return Err(
            "restored target does not contain the bounded legacy recommender branch".into(),
        );
    }

    fs::write(&target_path, &restored).map_err(|e| e.to_string())?;

    Ok(json!({
        "schema": "code-intel-compatibility-rollback-rehearsal.v1",
        "branchId": BRANCH_ID,
        "target": target_path.to_string_lossy(),
        "sourceRevision": source_revision,
        "rehearsal": rehearsal,
        "changedFiles": [target_path.to_string_lossy()],
        "replacementChanged": false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn scratch_dir(name: &str) -> PathBuf {
        let nonce = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "recommender-restore-test-{name}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn restore_rehearsal_refuses_an_existing_root() {
        let dir = scratch_dir("existing-root");
        fs::create_dir_all(dir.join("run-code-intel.ps1").parent().unwrap()).unwrap();
        fs::write(dir.join("run-code-intel.ps1"), "irrelevant").unwrap();
        let rehearsal_root = dir.join("rehearsal");
        fs::create_dir_all(&rehearsal_root).unwrap();
        let result = restore_legacy_branch(
            &dir,
            RestoreMode::Rehearsal(rehearsal_root),
            crate::recommender_retirement_shared::DEFAULT_SOURCE_REVISION,
        );
        assert!(result.is_err());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn restore_refuses_when_run_code_intel_is_missing() {
        let dir = scratch_dir("missing-run-file");
        fs::create_dir_all(&dir).unwrap();
        let result = restore_legacy_branch(
            &dir,
            RestoreMode::Rehearsal(dir.join("rehearsal")),
            crate::recommender_retirement_shared::DEFAULT_SOURCE_REVISION,
        );
        assert!(result
            .unwrap_err()
            .contains("run-code-intel.ps1 is missing"));
        fs::remove_dir_all(&dir).ok();
    }

    /// Real test against this repository's actual git history and current
    /// `legacy/run-code-intel.ps1` -- proves the ported literal-block
    /// extraction still finds the same markers PowerShell's regex did,
    /// against the real file, not a synthetic fixture.
    #[test]
    fn restore_rehearsal_recovers_the_real_legacy_branch_from_git_history() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../legacy");
        if !repo_root.join("run-code-intel.ps1").is_file() {
            eprintln!("skipping: legacy/run-code-intel.ps1 not present in this checkout");
            return;
        }
        let rehearsal_root = scratch_dir("real-history-rehearsal");
        let result = restore_legacy_branch(
            &repo_root,
            RestoreMode::Rehearsal(rehearsal_root.clone()),
            crate::recommender_retirement_shared::DEFAULT_SOURCE_REVISION,
        );
        match &result {
            Ok(value) => {
                assert_eq!(value["rehearsal"], true);
                assert_eq!(value["replacementChanged"], false);
                let restored =
                    fs::read_to_string(rehearsal_root.join("run-code-intel.ps1")).unwrap();
                assert!(restored.contains("function Invoke-WorkflowStackDetector"));
            }
            Err(message) => panic!("rollback rehearsal against real history failed: {message}"),
        }
        fs::remove_dir_all(&rehearsal_root).ok();
    }
}
