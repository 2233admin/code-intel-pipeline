//! The four per-signal computations — diff shape, test asymmetry, bug-magnet
//! overlap, churn overlap — each folded into a `[0,1]` subscore, plus the
//! per-file detail rows that back the report's `files` array. `score_files`
//! is the entry point the parent module's `execute` calls once per scored
//! item (the target and each `--sample` baseline commit).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{json, Value};

use super::git::file_commit_history;
use super::scoring::combine;
use super::{
    BugMagnet, Churn, DiffShape, FileDiff, Scored, TestAsymmetry, BUG_MAGNET_CAP, CHURN_CAP,
    CHURN_WINDOW_DAYS, FILES_TOUCHED_CAP, LINES_CHANGED_CAP,
};

pub(super) fn score_files(
    repo: &Path,
    files: &[FileDiff],
    anchor_unix: i64,
    exclude: &BTreeSet<String>,
) -> Scored {
    let diff = diff_shape(files);
    let asymmetry = test_asymmetry_signal(files);
    let paths: Vec<String> = files.iter().map(|(_, _, path)| path.clone()).collect();
    let history = file_commit_history(repo, &paths, anchor_unix, exclude);
    let bug_magnet = bug_magnet_signal(&paths, &history);
    let churn = churn_signal(&paths, &history, anchor_unix);
    let score = combine(&diff, &asymmetry, &bug_magnet, &churn);
    let scored_files = build_scored_files(files, &history, anchor_unix);
    Scored {
        score,
        diff,
        test_asymmetry: asymmetry,
        bug_magnet,
        churn,
        files: scored_files,
    }
}

fn diff_shape(files: &[FileDiff]) -> DiffShape {
    let files_touched = files.len();
    let insertions: i64 = files.iter().map(|(ins, _, _)| *ins).sum();
    let deletions: i64 = files.iter().map(|(_, del, _)| *del).sum();
    let lines_changed = insertions + deletions;
    let max_file_lines = files
        .iter()
        .map(|(ins, del, _)| ins + del)
        .max()
        .unwrap_or(0);
    let max_file_share = if lines_changed > 0 {
        max_file_lines as f64 / lines_changed as f64
    } else {
        0.0
    };
    let size = (files_touched as f64 / FILES_TOUCHED_CAP).min(1.0);
    let magnitude = (lines_changed as f64 / LINES_CHANGED_CAP).min(1.0);
    // Breadth (size) and magnitude flag a change that is big or wide;
    // concentration flags the complementary shape, a change that dumps
    // nearly everything into one file. Averaging the three means a diff has
    // to be unremarkable on all three axes to score low, not just small on
    // whichever one happens to be checked first.
    let subscore = (size + magnitude + max_file_share) / 3.0;
    DiffShape {
        files_touched,
        insertions,
        deletions,
        lines_changed,
        max_file_share,
        subscore,
    }
}

/// `crates/**/src/**`: a source file at least two directories under
/// `crates/`, with `src` somewhere in between and at least one path segment
/// after it.
pub(super) fn is_source_file(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.first() != Some(&"crates") {
        return false;
    }
    parts
        .iter()
        .position(|segment| *segment == "src")
        .is_some_and(|index| index + 1 < parts.len())
}

/// `tests/**`, `*_test.*`, or `test_*`, matched on path segments and
/// filename rather than a literal glob engine (no new dependency for three
/// simple patterns).
pub(super) fn is_test_file(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.contains(&"tests") {
        return true;
    }
    let filename = parts.last().copied().unwrap_or("");
    filename.starts_with("test_") || file_stem(filename).ends_with("_test")
}

fn file_stem(filename: &str) -> &str {
    match filename.rfind('.') {
        Some(index) if index > 0 => &filename[..index],
        _ => filename,
    }
}

fn test_asymmetry_signal(files: &[FileDiff]) -> TestAsymmetry {
    let source_files_changed = files
        .iter()
        .filter(|(_, _, path)| is_source_file(path))
        .count();
    let test_files_changed = files
        .iter()
        .filter(|(_, _, path)| is_test_file(path))
        .count();
    let asymmetric = source_files_changed > 0 && test_files_changed == 0;
    TestAsymmetry {
        source_files_changed,
        test_files_changed,
        asymmetric,
        subscore: if asymmetric { 1.0 } else { 0.0 },
    }
}

/// Matches `/fix|修复|修正/i` without a regex dependency: "fix" has no case
/// variants worth folding beyond ASCII lowercasing, and the two Chinese
/// markers have no case at all.
pub(super) fn looks_like_fix_subject(subject: &str) -> bool {
    subject.to_lowercase().contains("fix") || subject.contains("修复") || subject.contains("修正")
}

fn bug_magnet_signal(
    paths: &[String],
    history: &BTreeMap<String, Vec<(i64, String)>>,
) -> BugMagnet {
    let mut total_fix_commits = 0usize;
    let mut files_with_history = 0usize;
    for path in paths {
        let count = history
            .get(path)
            .map(|entries| {
                entries
                    .iter()
                    .filter(|(_, subject)| looks_like_fix_subject(subject))
                    .count()
            })
            .unwrap_or(0);
        if count > 0 {
            files_with_history += 1;
        }
        total_fix_commits += count;
    }
    BugMagnet {
        total_fix_commits,
        files_with_history,
        subscore: (total_fix_commits as f64 / BUG_MAGNET_CAP).min(1.0),
    }
}

fn churn_signal(
    paths: &[String],
    history: &BTreeMap<String, Vec<(i64, String)>>,
    anchor_unix: i64,
) -> Churn {
    let since = anchor_unix - CHURN_WINDOW_DAYS * 86_400;
    let mut total_commits = 0usize;
    for path in paths {
        if let Some(entries) = history.get(path) {
            total_commits += entries
                .iter()
                .filter(|(timestamp, _)| *timestamp >= since)
                .count();
        }
    }
    Churn {
        total_commits,
        subscore: (total_commits as f64 / CHURN_CAP).min(1.0),
    }
}

fn build_scored_files(
    files: &[FileDiff],
    history: &BTreeMap<String, Vec<(i64, String)>>,
    anchor_unix: i64,
) -> Vec<Value> {
    let since_churn = anchor_unix - CHURN_WINDOW_DAYS * 86_400;
    files
        .iter()
        .map(|(insertions, deletions, path)| {
            let entries = history.get(path).map(Vec::as_slice).unwrap_or(&[]);
            let bug_fix_commits = entries
                .iter()
                .filter(|(_, subject)| looks_like_fix_subject(subject))
                .count();
            let churn_commits = entries
                .iter()
                .filter(|(timestamp, _)| *timestamp >= since_churn)
                .count();
            json!({
                "path": path,
                "insertions": insertions,
                "deletions": deletions,
                "isSourceFile": is_source_file(path),
                "isTestFile": is_test_file(path),
                "bugFixCommits180d": bug_fix_commits,
                "churnCommits90d": churn_commits,
            })
        })
        .collect()
}
