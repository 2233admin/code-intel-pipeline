//! Enforcement the compatibility retirement gate never had: proves every
//! legacy branch tracked under `orchestration/retirements/*` is either
//! still present in the tree, or has a `status.json` that says `retired:
//! true`. Closes the gap E09 (`orchestration/retirements/e09-doctor-wrapper`)
//! exposed -- that branch's code was removed by an ordinary commit while its
//! own gate decision still read `blocked`, and nothing caught it before
//! merge. The gate itself never deletes code (see
//! `docs/compatibility-retire-recommender-branch.md`); this guard is the
//! missing check that a deletion actually happened through an authorized
//! retirement, not around one.
//!
//! Each packet's own `compatibility-retirement-deletion-diff.json` already
//! records exactly which lines its legacy branch occupies (the
//! `replayable-delete-only-v1` patch's `deletedLines`, per hunk) -- that is
//! reused here as the presence oracle instead of re-deriving per-branch
//! marker patterns. A hunk's `deletedLines`, joined by `\n`, must still be a
//! literal substring of the current file unless the packet is retired.

use std::fs;
use std::path::Path;

use serde_json::Value;

pub(crate) struct Violation {
    pub(crate) packet: String,
    pub(crate) file: String,
    pub(crate) message: String,
}

/// Scan every packet directory under `retirements_dir`. Returns one
/// [`Violation`] per legacy branch that has disappeared from `repo_root`
/// without its packet's `status.json` saying `retired: true`. An empty
/// result means every tracked branch is either still present or was
/// retired through the authorized path.
pub(crate) fn check(retirements_dir: &Path, repo_root: &Path) -> Result<Vec<Violation>, String> {
    let mut violations = Vec::new();
    let entries = fs::read_dir(retirements_dir)
        .map_err(|e| format!("read {}: {e}", retirements_dir.display()))?;
    let mut packet_dirs: Vec<_> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .collect();
    packet_dirs.sort_by_key(|entry| entry.file_name());

    for entry in packet_dirs {
        let packet_dir = entry.path();
        let packet_name = entry.file_name().to_string_lossy().into_owned();
        let status_path = packet_dir.join("status.json");
        let diff_path = packet_dir.join("compatibility-retirement-deletion-diff.json");
        if !status_path.is_file() || !diff_path.is_file() {
            // A packet mid-generation (no status/diff yet) makes no claim
            // about the tree and is not this guard's concern.
            continue;
        }
        let status: Value = serde_json::from_slice(
            &fs::read(&status_path).map_err(|e| format!("read {}: {e}", status_path.display()))?,
        )
        .map_err(|e| format!("parse {}: {e}", status_path.display()))?;
        if status["retired"].as_bool() == Some(true) {
            continue;
        }
        let diff: Value = serde_json::from_slice(
            &fs::read(&diff_path).map_err(|e| format!("read {}: {e}", diff_path.display()))?,
        )
        .map_err(|e| format!("parse {}: {e}", diff_path.display()))?;
        let files = diff["patch"]["files"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for file in files {
            let relative_path = file["path"].as_str().unwrap_or("");
            if relative_path.is_empty() {
                continue;
            }
            let absolute_path = repo_root.join(relative_path);
            let current = match fs::read_to_string(&absolute_path) {
                Ok(text) => text,
                Err(_) => {
                    violations.push(Violation {
                        packet: packet_name.clone(),
                        file: relative_path.to_string(),
                        message: format!(
                            "legacy file is gone entirely ({} not found) but status.json does not say retired: true",
                            absolute_path.display()
                        ),
                    });
                    continue;
                }
            };
            let normalized = current.replace("\r\n", "\n").replace('\r', "\n");
            let hunks = file["hunks"].as_array().cloned().unwrap_or_default();
            let mut present_count = 0usize;
            let mut total_count = 0usize;
            for hunk in &hunks {
                let deleted_lines: Vec<String> = hunk["deletedLines"]
                    .as_array()
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                if deleted_lines.is_empty() {
                    continue;
                }
                total_count += 1;
                let needle = deleted_lines.join("\n");
                if normalized.contains(&needle) {
                    present_count += 1;
                }
            }
            // A branch mid-retirement can legitimately shed hunks one at a
            // time -- see e05-publication's own
            // Test-PublicationRetirementBoundary.ps1, which explicitly
            // tolerates "(1,0) marker removed, staging still present" as a
            // current, expected architecture state. Only the branch's
            // total, silent disappearance (zero of its tracked hunks left)
            // is what E09 exposed and what this guard exists to catch.
            if total_count > 0 && present_count == 0 {
                violations.push(Violation {
                    packet: packet_name.clone(),
                    file: relative_path.to_string(),
                    message: format!(
                        "the tracked legacy branch is entirely gone from {relative_path} ({total_count} of {total_count} hunks removed) but status.json does not say retired: true -- either this deletion is an authorized retirement (update status.json through the real E00/E01 flow) or the branch must be restored"
                    ),
                });
            }
        }
    }
    Ok(violations)
}

/// Findings already tracked, not silently accepted -- mirrors
/// legacy/scripts/tests/test-retirement-packets.ps1's `$KnownBlocked`
/// pattern: an entry here must match its exact current message, so this
/// list cannot rot silently. If e03-provider-preflight's status.json is
/// later corrected to `retired: true` through the real out-of-band
/// documentation this needs (mirroring e09-doctor-wrapper's shape), this
/// entry stops matching and `run_raw_inner` fails loud until the entry is
/// deliberately removed. Tracked in issue #341's discussion, discovered by
/// this guard on 2026-08-25: e03's legacy branch was actually removed by
/// 42de0635 (the same commit that removed e02's), the same undeclared
/// bypass pattern e09 already documents, just never written down for e03.
fn known_findings() -> &'static [(&'static str, &'static str)] {
    &[(
        "e03-provider-preflight",
        "the tracked legacy branch is entirely gone from run-code-intel.ps1 (1 of 1 hunks removed) but status.json does not say retired: true -- either this deletion is an authorized retirement (update status.json through the real E00/E01 flow) or the branch must be restored",
    )]
}

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    match run_raw_inner(raw) {
        Ok(true) => {
            println!("{{\"ok\":true,\"violations\":[]}}");
            0
        }
        Ok(false) => 74,
        Err(message) => {
            eprintln!("error: {message}");
            65
        }
    }
}

fn run_raw_inner(raw: &[String]) -> Result<bool, String> {
    let mut repo_root = None;
    let mut retirements_dir = None;
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--repo-root" if i + 1 < raw.len() => repo_root = Some(raw[i + 1].clone()),
            "--retirements-dir" if i + 1 < raw.len() => retirements_dir = Some(raw[i + 1].clone()),
            other => return Err(format!("unknown retirement guard argument: {other}")),
        }
        i += 2;
    }
    let repo_root = repo_root.ok_or("--repo-root is required")?;
    let retirements_dir =
        retirements_dir.unwrap_or_else(|| "orchestration/retirements".to_string());
    let violations = check(Path::new(&retirements_dir), Path::new(&repo_root))?;

    let known = known_findings();
    let mut unexpected = Vec::new();
    let mut matched_known = vec![false; known.len()];
    for violation in &violations {
        match known.iter().position(|(packet, message)| {
            *packet == violation.packet && *message == violation.message
        }) {
            Some(index) => matched_known[index] = true,
            None => unexpected.push(violation),
        }
    }
    if !unexpected.is_empty() {
        for violation in &unexpected {
            eprintln!(
                "error: {} ({}): {}",
                violation.packet, violation.file, violation.message
            );
        }
        return Ok(false);
    }
    for (index, (packet, _)) in known.iter().enumerate() {
        if !matched_known[index] {
            return Err(format!(
                "known_findings() entry for {packet} no longer matches any real violation -- \
                 either it was fixed (remove the entry) or its shape changed (update the entry); \
                 a stale allowlist entry cannot be trusted to mean the finding is still open"
            ));
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let nonce = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "retirement-guard-test-{name}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn write_packet(
        retirements_dir: &Path,
        name: &str,
        retired: bool,
        file_path: &str,
        deleted_lines: &[&str],
    ) {
        let dir = retirements_dir.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("status.json"),
            serde_json::to_vec(&json!({"retired": retired})).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("compatibility-retirement-deletion-diff.json"),
            serde_json::to_vec(&json!({
                "patch": {
                    "files": [{
                        "path": file_path,
                        "hunks": [{"deletedLines": deleted_lines}]
                    }]
                }
            }))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn passes_when_the_tracked_branch_is_still_present() {
        let dir = scratch_dir("still-present");
        let retirements = dir.join("retirements");
        let repo_root = dir.join("repo");
        fs::create_dir_all(&repo_root).unwrap();
        write_packet(
            &retirements,
            "e-example",
            false,
            "run-code-intel.ps1",
            &["function LegacyThing {", "    return 1", "}"],
        );
        fs::write(
            repo_root.join("run-code-intel.ps1"),
            "before\nfunction LegacyThing {\n    return 1\n}\nafter",
        )
        .unwrap();
        let violations = check(&retirements, &repo_root).unwrap();
        assert!(violations.is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flags_a_disappeared_branch_whose_packet_is_not_retired() {
        let dir = scratch_dir("disappeared-not-retired");
        let retirements = dir.join("retirements");
        let repo_root = dir.join("repo");
        fs::create_dir_all(&repo_root).unwrap();
        write_packet(
            &retirements,
            "e-example",
            false,
            "run-code-intel.ps1",
            &["function LegacyThing {", "    return 1", "}"],
        );
        fs::write(repo_root.join("run-code-intel.ps1"), "before\nafter").unwrap();
        let violations = check(&retirements, &repo_root).unwrap();
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].packet, "e-example");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn does_not_flag_a_disappeared_branch_whose_packet_is_retired() {
        let dir = scratch_dir("disappeared-retired");
        let retirements = dir.join("retirements");
        let repo_root = dir.join("repo");
        fs::create_dir_all(&repo_root).unwrap();
        write_packet(
            &retirements,
            "e-example",
            true,
            "run-code-intel.ps1",
            &["function LegacyThing {", "    return 1", "}"],
        );
        fs::write(repo_root.join("run-code-intel.ps1"), "before\nafter").unwrap();
        let violations = check(&retirements, &repo_root).unwrap();
        assert!(violations.is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flags_a_deleted_file_whose_packet_is_not_retired() {
        let dir = scratch_dir("deleted-file");
        let retirements = dir.join("retirements");
        let repo_root = dir.join("repo");
        fs::create_dir_all(&repo_root).unwrap();
        write_packet(
            &retirements,
            "e-example",
            false,
            "invoke-code-intel.ps1",
            &["function LegacyThing {}"],
        );
        // repo_root exists but the file itself does not.
        let violations = check(&retirements, &repo_root).unwrap();
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("gone entirely"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn skips_packets_still_mid_generation() {
        let dir = scratch_dir("mid-generation");
        let retirements = dir.join("retirements");
        let repo_root = dir.join("repo");
        fs::create_dir_all(&repo_root).unwrap();
        fs::create_dir_all(retirements.join("e-partial")).unwrap();
        // no status.json / deletion-diff.json written yet
        let violations = check(&retirements, &repo_root).unwrap();
        assert!(violations.is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    /// Real test against this repository's actual retirement packets.
    /// e05-publication is expected to be clean: its own
    /// Test-PublicationRetirementBoundary.ps1 already documents a
    /// tolerated partial state (one hunk retired, one still present) as
    /// current architecture progress, and this guard's zero-of-N rule
    /// agrees. e03-provider-preflight is a genuine finding, not a test
    /// bug: its tracked legacy branch is entirely gone from
    /// run-code-intel.ps1 (confirmed separately via `grep -c
    /// test-code-intel-provider legacy/run-code-intel.ps1` => 0) while its
    /// status.json still reads `retired: false` -- a second, previously
    /// undetected instance of exactly the E09 bypass pattern.
    #[test]
    fn matches_the_known_real_repository_state() {
        let retirements_dir =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../orchestration/retirements");
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../legacy");
        if !retirements_dir.is_dir() {
            eprintln!("skipping: orchestration/retirements not present in this checkout");
            return;
        }
        let violations = check(&retirements_dir, &repo_root).unwrap();
        let packets: Vec<&str> = violations.iter().map(|v| v.packet.as_str()).collect();
        assert_eq!(
            packets,
            vec!["e03-provider-preflight"],
            "expected exactly the known e03-provider-preflight undeclared-bypass finding, got: {}",
            violations
                .iter()
                .map(|v| format!("{} ({}): {}", v.packet, v.file, v.message))
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
}
