//! `diagnosis.hospital`'s R1.4 ghost-path guard.
//!
//! Split out of `hospital_diagnosis.rs`: R1.4 (issue #47's ghost-surgery-target
//! fix) added this check plus its fixture-backed tests, which pushed that file
//! past the repository's own god-file threshold (loc > 800 or functions > 25
//! with loc > 400). The seam is the concern boundary the doc comment below
//! already describes — filesystem-existence verification of an admitted
//! evidence path — not an arbitrary line-count cut.

use std::path::Path;

use crate::adapter_contract::AdapterError;

/// R1.4: a surgery target the machine is about to publish must resolve to a
/// real file inside the repository this run scanned. Admitted evidence is
/// opaque JSON from an external producer (Sentrux rule violations, the
/// native-code enrichment's `topTarget`) — nothing upstream of this function
/// checks that the path it names still exists in the snapshot it claims to
/// describe. A target this cannot verify is a defect in the pipeline's own
/// evidence, not a fact about the repository, so it must never reach
/// `primary_target.file` unexamined: this is the reject-and-fail-closed half
/// of that guard (`execute` skips the call entirely when no `repoPath` was
/// supplied, so absence of a check is never confused with a passed one).
///
/// A path is rejected before it ever touches the filesystem if it is empty or
/// has any non-`Normal` component (`..`, a root, a Windows drive prefix): a
/// `..`-relative or absolute candidate joined onto `repo` can walk or jump
/// outside it entirely, which would make this check answer a different
/// question than "does this path exist inside the scanned repository".
pub(crate) fn verify_surgery_target_exists(repo: &Path, target: &str) -> Result<(), AdapterError> {
    let candidate = Path::new(target);
    let is_repo_relative = !target.is_empty()
        && candidate
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)));
    if !is_repo_relative || !repo.join(candidate).is_file() {
        return Err(AdapterError::Contract(format!(
            "admitted evidence names a surgery target that does not exist in the scanned repository snapshot: {target}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// R1.4 fixture repository: a directory with exactly one real file, used
    /// to distinguish "exists" from "does not exist" without touching the
    /// crate's own tree.
    struct FixtureRepo(std::path::PathBuf);

    impl FixtureRepo {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "code-intel-b09-ghost-path-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(path.join("src")).unwrap();
            fs::write(path.join("src/real.rs"), b"// present\n").unwrap();
            Self(path)
        }
    }

    impl Drop for FixtureRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn verify_surgery_target_exists_accepts_a_real_repo_relative_file() {
        let repo = FixtureRepo::new();
        assert!(verify_surgery_target_exists(&repo.0, "src/real.rs").is_ok());
    }

    #[test]
    fn verify_surgery_target_exists_rejects_a_path_absent_from_the_snapshot() {
        // The historical case this guards: a surgery-plan once named a path
        // from a different worktree entirely
        // (`.claude/worktrees/project-bug-investigation-0d24d9/...`) — a
        // plausible-looking relative path that simply is not in the repo the
        // run actually scanned.
        let repo = FixtureRepo::new();
        let error = verify_surgery_target_exists(
            &repo.0,
            ".claude/worktrees/project-bug-investigation-0d24d9/src/missing.rs",
        )
        .unwrap_err();
        assert!(matches!(error, AdapterError::Contract(_)));
    }

    #[test]
    fn verify_surgery_target_exists_rejects_traversal_even_if_it_would_resolve() {
        let repo = FixtureRepo::new();
        // Escapes `repo` via `..` rather than staying inside it; must be
        // rejected on shape alone, independent of what happens to sit there.
        let outside = repo.0.parent().unwrap().file_name().unwrap();
        let traversal = format!("../{}/nonexistent-marker-file", outside.to_string_lossy());
        assert!(verify_surgery_target_exists(&repo.0, &traversal).is_err());
    }

    #[test]
    fn verify_surgery_target_exists_rejects_an_absolute_path() {
        let repo = FixtureRepo::new();
        let absolute = repo.0.join("src/real.rs");
        // A real file, but named absolutely: `Path::join` would let an
        // absolute candidate replace `repo` outright rather than resolve
        // inside it, so this must fail on shape before that substitution
        // ever happens.
        assert!(verify_surgery_target_exists(&repo.0, &absolute.to_string_lossy()).is_err());
    }

    #[test]
    fn verify_surgery_target_exists_rejects_an_empty_target() {
        let repo = FixtureRepo::new();
        assert!(verify_surgery_target_exists(&repo.0, "").is_err());
    }
}
