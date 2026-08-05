//! All `git` subprocess plumbing for `change risk`: resolving the
//! repository root, turning a revspec into a diffable range, running
//! `git diff --numstat`, and walking commit history for the bug-magnet and
//! churn signals. Nothing here scores anything — it only shells out to
//! `git` and parses the output into the shared types from the parent
//! module.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::{Endpoint, FileDiff, FileHistory, RiskError, BUG_MAGNET_WINDOW_DAYS};

/// The empty tree object, constant in every Git repository. Used as the
/// diff base when a revspec resolves to a root commit with no parent to
/// diff against.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

pub(crate) fn resolve_repo_root() -> Result<PathBuf, RiskError> {
    let cwd = std::env::current_dir()
        .map_err(|error| RiskError::HostIo(format!("cannot resolve current directory: {error}")))?;
    resolve_repo_root_from(&cwd)
}

/// Resolves `start`'s Git repository root under the same "walk up to the
/// toplevel, fail closed if it is not a Git repository" contract
/// `resolve_repo_root` applies to the current directory. Backs an explicit
/// `--repo <path>` (issue #114): every sibling subcommand (`run execute
/// --repo`, `audit --repo`, `snapshot identity --repo`) takes an explicit
/// repo path rather than always trusting the CWD, and this closes that gap
/// for `change risk` without changing the CWD-derived default at all.
pub(crate) fn resolve_repo_root_from(start: &Path) -> Result<PathBuf, RiskError> {
    let output = crate::hardened_git::command(start)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|error| RiskError::HostIo(format!("cannot launch Git: {error}")))?;
    if !output.status.success() {
        return Err(RiskError::Contract(format!(
            "not a Git repository: {}",
            start.display()
        )));
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        return Err(RiskError::Contract(format!(
            "not a Git repository: {}",
            start.display()
        )));
    }
    Ok(PathBuf::from(path))
}

/// Splits a revspec into (range-to-diff, tip-to-anchor-on). A "A..B" or
/// "A...B" range is handed to `git diff` as-is (git parses either dot form
/// itself); a bare single commit has no diff of its own, so it is rewritten
/// as "<rev>^..<rev>" to diff it against its first parent.
pub(crate) fn resolve_endpoint(revspec: &str) -> Endpoint {
    let tip = tip_token(revspec).to_string();
    let range = if revspec.contains("..") {
        revspec.to_string()
    } else {
        format!("{revspec}^..{revspec}")
    };
    Endpoint { range, tip }
}

/// The right-hand side of a range ("A..B"/"A...B" -> "B", defaulting to
/// HEAD when omitted); a bare revspec with no ".." is already its own tip.
/// Git ref names cannot contain "..", so any ".." substring is unambiguously
/// a range operator, never part of a name.
pub(super) fn tip_token(revspec: &str) -> &str {
    if let Some(index) = revspec.rfind("..") {
        let after = revspec[index..].trim_start_matches('.');
        if after.is_empty() {
            "HEAD"
        } else {
            after
        }
    } else {
        revspec
    }
}

pub(crate) fn diff_stats(repo: &Path, range: &str) -> Result<Vec<FileDiff>, RiskError> {
    let primary = run_git_diff_numstat(repo, range);
    if primary.is_ok() {
        return primary;
    }
    // Likely a root commit with no parent on the left side of "X^..X";
    // retry against the empty tree so the very first commit in a
    // repository can still be scored.
    if let Some((left, right)) = range.split_once("..") {
        if let Some(root) = left.strip_suffix('^') {
            if root == right {
                let fallback = format!("{EMPTY_TREE}..{right}");
                return run_git_diff_numstat(repo, &fallback);
            }
        }
    }
    primary
}

fn run_git_diff_numstat(repo: &Path, range: &str) -> Result<Vec<FileDiff>, RiskError> {
    let output = crate::hardened_git::command(repo)
        .args([
            // Git C-quotes non-ASCII paths by default ("\346\226\207..."),
            // which would never match the repository-relative keys this
            // module looks paths up by. Applies to every parsed-path call
            // site, not cosmetic ones.
            "-c",
            "core.quotePath=false",
            "diff",
            "--numstat",
            "--no-renames",
            range,
        ])
        .output()
        .map_err(|error| RiskError::HostIo(format!("cannot launch Git: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(RiskError::Contract(if stderr.is_empty() {
            format!("cannot resolve change-risk revspec: {range}")
        } else {
            format!("cannot resolve change-risk revspec {range}: {stderr}")
        }));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_numstat_line)
        .collect())
}

fn parse_numstat_line(line: &str) -> Option<FileDiff> {
    let mut parts = line.splitn(3, '\t');
    let insertions = parts.next()?;
    let deletions = parts.next()?;
    let path = parts.next()?;
    let insertions = if insertions == "-" {
        0
    } else {
        insertions.parse().ok()?
    };
    let deletions = if deletions == "-" {
        0
    } else {
        deletions.parse().ok()?
    };
    let path = normalize_path(path);
    if path.is_empty() {
        return None;
    }
    Some((insertions, deletions, path))
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

pub(crate) fn commit_unix_time(repo: &Path, rev: &str) -> Result<i64, RiskError> {
    let output = crate::hardened_git::command(repo)
        .args(["log", "-1", "--format=%ct", rev])
        .output()
        .map_err(|error| RiskError::HostIo(format!("cannot launch Git: {error}")))?;
    if !output.status.success() {
        return Err(RiskError::HostIo(format!(
            "cannot resolve commit time for {rev}"
        )));
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<i64>()
        .map_err(|_| RiskError::HostIo(format!("unexpected git log output for {rev}")))
}

/// Every commit the diff at `range` was built from, so its own history walk
/// can exclude them (see the "Self-reference" module doc). Best-effort: a
/// failure here degrades to "nothing excluded" rather than erroring, since
/// `diff_stats` already proved the range itself resolves.
pub(crate) fn commits_in_range(repo: &Path, range: &str) -> Vec<String> {
    let range = rev_list_range(range);
    let Ok(output) = crate::hardened_git::command(repo)
        .args(["rev-list", &range])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

/// `git diff --numstat A...B` compares merge-base(A,B) to B — only B's
/// commits since it diverged from A. `git rev-list A...B`, in contrast,
/// returns the *symmetric* difference: commits unique to A as well as
/// commits unique to B. Left as-is, a three-dot `range` would make
/// `commits_in_range` return A's own history alongside B's, wrongly
/// excluding commits from bug-magnet/churn lookups that were never part of
/// the diff being scored. `git rev-list A..B` (two-dot) is the asymmetric
/// form — commits reachable from B but not from A — which is exactly the
/// "since merge-base" set `git diff A...B` compares against (merge-base is
/// necessarily an ancestor of A, so "not reachable from A" and "not
/// reachable from merge-base" select the same commits). Rewriting the first
/// `...` to `..` before `rev-list` therefore makes the exclusion set match
/// the diff precisely. Git ref names cannot contain "..", so the first
/// `...` substring is unambiguously the range operator, never part of a
/// name.
fn rev_list_range(range: &str) -> String {
    range.replacen("...", "..", 1)
}

/// The last `sample` non-merge commits reachable from `start`, each with its
/// own committer-date anchor, fetched in one call so scoring N samples costs
/// N additional git invocations rather than 2N.
pub(super) fn sample_history(repo: &Path, start: &str, sample: u32) -> Vec<(String, i64)> {
    if sample == 0 {
        return Vec::new();
    }
    let Ok(output) = crate::hardened_git::command(repo)
        .args([
            "log",
            "--no-merges",
            &format!("--max-count={sample}"),
            "--format=%H%x1f%ct",
            start,
        ])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, '\u{1f}');
            let sha = parts.next()?.trim().to_string();
            let timestamp = parts.next()?.trim().parse::<i64>().ok()?;
            (!sha.is_empty()).then_some((sha, timestamp))
        })
        .collect()
}

/// One commit as seen by [`walk_log`]: identity, committer date, subject,
/// and the touched paths that fell inside the queried path set.
pub(crate) struct LogCommit {
    pub(crate) hash: String,
    pub(crate) timestamp: i64,
    pub(crate) subject: String,
    pub(crate) paths: Vec<String>,
}

/// One `git log` call covering every touched file at once (mirrors the
/// churn walk in `sentrux_analysis::apply_git_log`): far cheaper than a
/// per-file subprocess. Both consumers below project from this single
/// parser rather than each parsing `--name-only` output their own way —
/// two readers of one format is exactly how the file counts in issue #148
/// C2 drifted apart.
///
/// Commits in `exclude` (the commit set the diff itself was built from) are
/// dropped so a change is never evidence of its own history. A git failure
/// degrades to an empty walk rather than an error: every caller already
/// proved the range resolves, and a missing history signal is honestly
/// reported as absent by the caller's own shape.
fn walk_log(
    repo: &Path,
    files: &[String],
    since_unix: i64,
    until_unix: i64,
    exclude: &BTreeSet<String>,
) -> Vec<LogCommit> {
    if files.is_empty() {
        return Vec::new();
    }
    let queried: BTreeSet<&String> = files.iter().collect();
    let Ok(output) = crate::hardened_git::command(repo)
        .args([
            // Same quotePath rule as run_git_diff_numstat: --name-only output
            // is matched against `files` keys and must arrive unquoted.
            "-c",
            "core.quotePath=false",
            "log",
            &format!("--since=@{since_unix}"),
            &format!("--until=@{until_unix}"),
            "--format=__CIRISK__%H%x1f%ct%x1f%s",
            "--name-only",
            "--",
        ])
        .args(files.iter())
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let mut commits: Vec<LogCommit> = Vec::new();
    let mut current: Option<LogCommit> = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(rest) = line.strip_prefix("__CIRISK__") {
            if let Some(finished) = current.take() {
                commits.push(finished);
            }
            let mut parts = rest.splitn(3, '\u{1f}');
            let hash = parts.next().unwrap_or("").to_string();
            let timestamp = parts.next().and_then(|value| value.parse::<i64>().ok());
            let subject = parts.next().unwrap_or("").to_string();
            current = match timestamp {
                Some(timestamp) if !exclude.contains(&hash) => Some(LogCommit {
                    hash,
                    timestamp,
                    subject,
                    paths: Vec::new(),
                }),
                _ => None,
            };
            continue;
        }
        let path = line.trim();
        if path.is_empty() {
            continue;
        }
        let Some(commit) = current.as_mut() else {
            continue;
        };
        let path = normalize_path(path);
        if queried.contains(&path) {
            commit.paths.push(path);
        }
    }
    if let Some(finished) = current.take() {
        commits.push(finished);
    }
    commits
}

/// Per-file commit history for the bug-magnet and churn signals. Windowed
/// to the wider of the two signal windows (bug-magnet, 180 days) since
/// churn (90 days) is a subset of it; callers slice the narrower window
/// back out client-side.
pub(crate) fn file_commit_history(
    repo: &Path,
    files: &[String],
    anchor_unix: i64,
    exclude: &BTreeSet<String>,
) -> FileHistory {
    let mut history: FileHistory = files
        .iter()
        .map(|path| (path.clone(), Vec::new()))
        .collect();
    let since_unix = anchor_unix - BUG_MAGNET_WINDOW_DAYS * 86_400;
    for commit in walk_log(repo, files, since_unix, anchor_unix, exclude) {
        for path in commit.paths {
            if let Some(entries) = history.get_mut(&path) {
                entries.push((commit.timestamp, commit.subject.clone()));
            }
        }
    }
    history
}

/// Commits in the co-change window that touched **two or more** of `files`,
/// each as the touched subset itself (sorted, deduplicated). A commit
/// touching one or zero of the changed files carries no coupling evidence
/// and is dropped here rather than counted as a zero-weight edge. Backs
/// `change agenda` (issue #150).
pub(crate) fn cochanging_commits(
    repo: &Path,
    files: &[String],
    anchor_unix: i64,
    window_days: i64,
    exclude: &BTreeSet<String>,
) -> Vec<LogCommit> {
    let since_unix = anchor_unix - window_days * 86_400;
    walk_log(repo, files, since_unix, anchor_unix, exclude)
        .into_iter()
        .filter_map(|mut commit| {
            commit.paths.sort();
            commit.paths.dedup();
            (commit.paths.len() >= 2).then_some(commit)
        })
        .collect()
}
