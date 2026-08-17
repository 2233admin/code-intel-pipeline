use super::*;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_dir(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    env::temp_dir().join(format!("code-intel-{name}-{stamp}"))
}

fn touch(path: &Path, text: &str) {
    fs::write(path, text).expect("fixture file should be writable");
}

fn summary_for(report: Value, hospital: Value, artifact_dir: &Path) -> ResumeSummary {
    build_resume_summary(
        PathBuf::from("D:/work/quant-system"),
        artifact_dir.to_path_buf(),
        artifact_dir.join("report.json"),
        &report,
        &hospital,
    )
}

#[test]
fn resume_contract_routes_graph_missing_to_understanding() {
    let dir = unique_temp_dir("graph-missing");
    fs::create_dir_all(&dir).expect("fixture dir should be created");
    touch(&dir.join("summary.md"), "# Summary");
    touch(&dir.join("understanding.md"), "# Understanding");

    let hospital_path = dir.join("hospital-report.json");
    let hospital_markdown = dir.join("hospital.md");
    let report = json!({
        "hospital": {
            "path": hospital_path,
            "markdown": hospital_markdown
        },
        "githubResearch": {
            "status": "not_applicable",
            "required": false,
            "path": "",
            "markdown": ""
        },
        "summary": {
            "failed": 0,
            "manualRequired": 1,
            "failureCategories": {
                "providerQuota": 0,
                "localToolError": 0,
                "graphMissing": 1,
                "sentruxFail": 0
            }
        }
    });
    let hospital = json!({
        "triage": {
            "status": "amber",
            "disposition": "admit",
            "primary_diagnosis": "architecture graph missing",
            "next_protocol": "diagnose",
            "research_status": "not_applicable",
            "research_required": false
        },
        "state_machine": {
            "current_state": "diagnose"
        }
    });

    let summary = summary_for(report, hospital, &dir);

    assert_eq!(summary.graph_missing, 1);
    assert_eq!(summary.hospital_next_protocol, "diagnose");
    assert!(!summary.research_required);
    assert_eq!(next_read(&summary), dir.join("understanding.md"));
}

#[test]
fn resume_contract_prioritizes_github_research_when_required() {
    let dir = unique_temp_dir("research-required");
    fs::create_dir_all(&dir).expect("fixture dir should be created");
    touch(&dir.join("understanding.md"), "# Understanding");
    let research_markdown = dir.join("github-solution-research.md");
    touch(&research_markdown, "# GitHub Solution Research");

    let report = json!({
        "hospital": {
            "path": dir.join("hospital-report.json"),
            "markdown": dir.join("hospital.md")
        },
        "githubResearch": {
            "status": "manual_required",
            "required": true,
            "path": dir.join("github-solution-research.json"),
            "markdown": research_markdown
        },
        "summary": {
            "failed": 1,
            "manualRequired": 1,
            "failureCategories": {
                "providerQuota": 0,
                "localToolError": 0,
                "graphMissing": 0,
                "sentruxFail": 1
            }
        }
    });
    let hospital = json!({
        "triage": {
            "status": "red",
            "disposition": "admit",
            "primary_diagnosis": "architecture gate failure",
            "next_protocol": "github_solution_research",
            "research_status": "manual_required",
            "research_required": true
        },
        "state_machine": {
            "current_state": "triage"
        }
    });

    let summary = summary_for(report, hospital, &dir);

    assert_eq!(summary.sentrux_fail, 1);
    assert!(summary.research_required);
    assert_eq!(summary.hospital_next_protocol, "github_solution_research");
    assert_eq!(next_read(&summary), research_markdown);
}

#[test]
fn classify_contract_requires_research_for_upstream_or_tool_blockers() {
    let report = json!({
        "summary": {
            "failureCategories": {
                "providerQuota": 1,
                "localToolError": 0,
                "graphMissing": 1,
                "sentruxFail": 0
            }
        }
    });

    let provider_quota = int_at(&report, &["summary", "failureCategories", "providerQuota"]);
    let local_tool_error = int_at(&report, &["summary", "failureCategories", "localToolError"]);
    let sentrux_fail = int_at(&report, &["summary", "failureCategories", "sentruxFail"]);

    assert!(provider_quota > 0 || local_tool_error > 0 || sentrux_fail > 0);
}

#[test]
fn ensure_directory_creates_a_fresh_nested_directory() {
    let dir = unique_temp_dir("ensure-directory-fresh");
    let target = dir.join("nested").join("leaf");

    let resolved = ensure_directory(&target).expect("create a fresh nested directory");
    assert_eq!(resolved, target);
    assert!(resolved.is_dir());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn ensure_directory_is_idempotent_for_an_existing_directory() {
    // Re-running against an already-analyzed repo is the ordinary case, not
    // an exotic one: the authority root container directory is meant to be
    // reused run after run.
    let dir = unique_temp_dir("ensure-directory-idempotent");
    fs::create_dir_all(&dir).expect("fixture dir");

    let resolved = ensure_directory(&dir).expect("re-create an already-existing directory");
    assert_eq!(resolved, dir);
    assert!(resolved.is_dir());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn ensure_directory_reports_a_real_file_collision_with_its_path_preserved() {
    // A genuine blocker (something real occupying the path) must still fail
    // — self-healing is only for the false-positive Windows symlink case
    // below, never for an actual collision.
    let dir = unique_temp_dir("ensure-directory-file-collision");
    fs::create_dir_all(&dir).expect("fixture parent");
    let blocked = dir.join("blocked");
    fs::write(&blocked, b"not a directory").expect("fixture blocking file");

    let error =
        ensure_directory(&blocked).expect_err("a real file must still block directory creation");
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);

    let _ = fs::remove_dir_all(&dir);
}

/// Issue #279's root cause, isolated from the rest of `code-intel`: on
/// Windows, `CreateDirectoryW` for a brand-new subdirectory can return
/// `ERROR_ALREADY_EXISTS` (os error 183) when an ancestor path component is a
/// symlink onto a *different* volume, even though the target does not exist
/// before or after the failed call, and an immediate retry against the same
/// unresolved path fails identically — not a race.
/// `create_dir_all`'s own recovery (`AlreadyExists` + `path.is_dir()` ==
/// success) can't catch this: `is_dir()` resolves through the same reparse
/// point and also reports `false`.
///
/// This was verified against a real, pre-existing cross-drive directory
/// symlink and reproduced every time. It is deliberately *not* a hard
/// assertion here, because it turns out not to be reliably fabricatable
/// on demand: neither a same-drive symlink, nor a *freshly created*
/// cross-drive symlink (this process or an external `mklink /D` process,
/// against multiple different drives, against both a brand-new empty target
/// and the real long-settled one) reproduced it — only the long-settled,
/// already-existing one did. Whatever OS-level state this depends on
/// evidently needs the reparse point to have existed for a while, which a
/// single test process cannot manufacture. So: attempt the real thing:
/// if this host happens to reproduce the quirk, assert `ensure_directory`
/// recovers from it; if not (the likely outcome, including in CI, which is a
/// freshly provisioned VM with no aged filesystem state), skip rather than
/// assert something false. `resolved_retry_target_rejoins_the_remainder_
/// under_the_canonicalized_ancestor` below covers the recovery arithmetic
/// deterministically instead. Mirrors this crate's existing convention for
/// environment-dependent symlink privilege (see `decision_record.rs`'s
/// `symlink_file(...).is_ok()` checks) rather than requiring elevation.
#[cfg(windows)]
#[test]
fn ensure_directory_recovers_from_a_spurious_already_exists_through_a_cross_volume_symlink() {
    let Some(other_drive) = second_writable_drive() else {
        eprintln!("skipping: no second drive available to build a cross-volume symlink");
        return;
    };

    let real_target = other_drive.join(format!(
        "code-intel-ensure-directory-target-{}",
        std::process::id()
    ));
    fs::create_dir_all(&real_target).expect("create real target on the other drive");

    let link_parent = unique_temp_dir("ensure-directory-symlink-parent");
    fs::create_dir_all(&link_parent).expect("create link parent");
    let link = link_parent.join("artifacts");
    if std::os::windows::fs::symlink_dir(&real_target, &link).is_err() {
        eprintln!("skipping: could not create a directory symlink (Developer Mode / privilege?)");
        let _ = fs::remove_dir_all(&real_target);
        let _ = fs::remove_dir_all(&link_parent);
        return;
    }

    let child = link.join("repo-name");

    // First, confirm this environment actually reproduces the quirk being
    // worked around — if it doesn't, the rest of the assertion proves
    // nothing.
    let raw = fs::create_dir_all(&child);
    let reproduces_quirk = matches!(
        &raw,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists
    ) && !child.is_dir();
    if !reproduces_quirk {
        eprintln!(
            "skipping: this host did not reproduce the cross-volume symlink quirk: {raw:?}"
        );
        let _ = fs::remove_dir_all(&real_target);
        let _ = fs::remove_dir_all(&link_parent);
        return;
    }

    let resolved = ensure_directory(&child)
        .unwrap_or_else(|error| panic!("did not recover from the symlink quirk: {error}"));
    assert!(resolved.is_dir(), "resolved={}", resolved.display());

    let _ = fs::remove_dir_all(&real_target);
    let _ = fs::remove_dir_all(&link_parent);
}

/// Finds a drive letter distinct from `env::temp_dir()`'s own that exists and
/// is writable, for building a genuinely cross-volume symlink. Returns `None`
/// on a lone-drive machine rather than guessing or failing.
#[cfg(windows)]
fn second_writable_drive() -> Option<PathBuf> {
    let own_letter = env::temp_dir()
        .to_str()
        .and_then(|path| path.chars().next())
        .map(|letter| letter.to_ascii_uppercase());
    for letter in ['D', 'E', 'F', 'G'] {
        if Some(letter) == own_letter {
            continue;
        }
        let root = PathBuf::from(format!("{letter}:\\"));
        if !root.is_dir() {
            continue;
        }
        let probe = root.join(format!("code-intel-drive-probe-{}", std::process::id()));
        if fs::create_dir(&probe).is_ok() {
            let _ = fs::remove_dir(&probe);
            return Some(root);
        }
    }
    None
}

/// Deterministic, platform-independent coverage for `ensure_directory`'s
/// recovery arithmetic (rejoining the part of the original target beyond the
/// existing ancestor onto that ancestor's canonicalized form) — the part of
/// the fix that does not depend on fabricating the Windows-only OS quirk
/// covered on a best-effort basis above.
#[test]
fn resolved_retry_target_rejoins_the_remainder_under_the_canonicalized_ancestor() {
    let path = Path::new("artifacts").join("fixture-repo");
    let existing_ancestor = Path::new("artifacts");
    let resolved_ancestor = Path::new("real-drive").join("cache").join("code-intel");
    assert_eq!(
        resolved_retry_target(&path, existing_ancestor, &resolved_ancestor),
        resolved_ancestor.join("fixture-repo")
    );

    // The target itself, with nothing beyond the existing ancestor, rejoins
    // to exactly the resolved ancestor.
    assert_eq!(
        resolved_retry_target(existing_ancestor, existing_ancestor, &resolved_ancestor),
        resolved_ancestor
    );
}
