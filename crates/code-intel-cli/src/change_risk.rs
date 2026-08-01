//! `code-intel change risk <revspec>` — deterministic, git-only defect-risk
//! score for a commit or range, plus that score's percentile against recent
//! repository history.
//!
//! v1 scoring is intentionally simple: four signals (diff shape, test
//! asymmetry, bug-magnet overlap, churn overlap), each git-derived, combined
//! with fixed weights documented next to their `const`s below. No index, no
//! network, no LLM — this exists so a PR gate can run before any of those
//! are available or trusted (see `.github/workflows/pr-gate.yml`).
//!
//! ## Anchoring
//!
//! Every signal that depends on "how many days back" is anchored to the
//! *commit being scored's own committer date*, never to wall-clock "now".
//! This matters most for `--sample`: the percentile baseline scores the
//! last N historical commits, and if their 90/180-day windows were all
//! measured against today, an old sampled commit would show near-zero churn
//! and bug-magnet activity purely because its own era has aged out of a
//! now-anchored window — not because the file was actually quiet at the
//! time. That would systematically deflate older baseline scores and make
//! the target look riskier than it is by comparison. Anchoring each scored
//! item (the target and every baseline sample) to its own tip commit's date
//! keeps the comparison apples-to-apples and, as a side effect, makes the
//! tool's output for a fixed revspec on a fixed repository state a fixed
//! answer forever: re-running this a year later reproduces the same score.
//!
//! ## Self-reference
//!
//! A commit inside the scored range would otherwise count toward its own
//! file's bug-magnet/churn tally — it is, after all, a commit touching that
//! file inside the window. `commits_in_range` computes the exact commit set
//! the diff was built from, and `file_commit_history` excludes it by hash,
//! so a change is never used as evidence of its own riskiness.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

#[path = "hardened_git.rs"]
mod hardened_git;

const USAGE: &str = "usage: change risk <revspec> [--sample <N>] [--format json|text]";

/// Baseline size when `--sample` is not given: enough commits to form a
/// stable percentile without walking arbitrarily deep into history on every
/// PR gate run.
const DEFAULT_SAMPLE: u32 = 50;

/// Trailing window for the bug-magnet signal. Roughly two release quarters:
/// long enough to see a file's recent track record, short enough that a fix
/// from years ago (plausibly against now-deleted code) does not haunt the
/// score forever.
const BUG_MAGNET_WINDOW_DAYS: i64 = 180;

/// Trailing window for the churn signal. Half the bug-magnet window: churn
/// counts every commit, not just fixes, so it is a noisier, faster-moving
/// signal — a shorter lookback keeps it reflecting what is hot right now
/// rather than diluted by a file's quieter history months back.
const CHURN_WINDOW_DAYS: i64 = 90;

/// Files touched beyond this already saturate the "how broad is this
/// change" component of diff shape; a 40-file mechanical rename is not 4x
/// riskier than a 10-file one by file count alone.
const FILES_TOUCHED_CAP: f64 = 20.0;

/// Changed lines (insertions + deletions) beyond this already saturate the
/// "how big is this change" component. Set well above a typical
/// single-concern commit so routine large-but-ordinary diffs do not auto-max
/// the signal, while genuinely sprawling changes still read as large.
const LINES_CHANGED_CAP: f64 = 600.0;

/// Combined fix-commit count across the touched files beyond this already
/// marks the area as fragile; more history does not need to push the
/// bug-magnet signal past saturation.
const BUG_MAGNET_CAP: f64 = 6.0;

/// Combined commit-touch count across the touched files beyond this already
/// marks the area as actively hot; more does not need to score higher.
const CHURN_CAP: f64 = 20.0;

/// Diff-shape weight: breadth + magnitude + single-file concentration. The
/// classic "large, sprawling changes correlate with defects" signal from
/// change-risk literature, and the cheapest to compute honestly from git
/// alone.
const WEIGHT_DIFF_SHAPE: f64 = 30.0;
/// Test-asymmetry weight: source changed with no matching test file touched
/// means nobody proved the change still works. The largest weight of any
/// single binary signal, because it is the strongest, most direct predictor
/// available here.
const WEIGHT_TEST_ASYMMETRY: f64 = 25.0;
/// Bug-magnet weight: touching files with a recent history of fix commits
/// means this change lands in territory that has needed correction before.
const WEIGHT_BUG_MAGNET: f64 = 25.0;
/// Churn weight: touching files under heavy recent edit activity means more
/// moving parts, less settled invariants, and more chance of colliding with
/// another change in flight.
const WEIGHT_CHURN: f64 = 20.0;

/// The empty tree object, constant in every Git repository. Used as the
/// diff base when a revspec resolves to a root commit with no parent to
/// diff against.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    match Cli::parse(raw).and_then(run) {
        Ok((value, format)) => {
            print_result(&value, format);
            0
        }
        Err(RiskError::Contract(message)) => {
            eprintln!("{message}");
            65
        }
        Err(RiskError::HostIo(message)) => {
            eprintln!("{message}");
            74
        }
    }
}

#[derive(Debug)]
enum RiskError {
    Contract(String),
    HostIo(String),
}

#[derive(Clone, Copy)]
enum Format {
    Json,
    Text,
}

struct Cli {
    revspec: String,
    sample: u32,
    format: Format,
}

impl Cli {
    fn parse(raw: &[String]) -> Result<Self, RiskError> {
        if raw.first().map(String::as_str) != Some("risk") {
            return Err(RiskError::Contract(USAGE.into()));
        }
        let mut revspec: Option<String> = None;
        let mut sample: Option<u32> = None;
        let mut format: Option<Format> = None;
        let mut index = 1;
        while index < raw.len() {
            let token = raw[index].as_str();
            match token {
                "--sample" => {
                    let value = raw
                        .get(index + 1)
                        .filter(|value| !value.is_empty() && !value.starts_with("--"))
                        .ok_or_else(|| RiskError::Contract("--sample requires one value".into()))?;
                    let parsed: u32 = value.parse().map_err(|_| {
                        RiskError::Contract(format!(
                            "--sample must be a non-negative integer: {value}"
                        ))
                    })?;
                    set_once(&mut sample, parsed, "--sample")?;
                    index += 2;
                }
                "--format" => {
                    let value = raw
                        .get(index + 1)
                        .filter(|value| !value.is_empty() && !value.starts_with("--"))
                        .ok_or_else(|| RiskError::Contract("--format requires one value".into()))?;
                    let parsed = match value.as_str() {
                        "json" => Format::Json,
                        "text" => Format::Text,
                        other => {
                            return Err(RiskError::Contract(format!(
                                "--format must be json or text: {other}"
                            )))
                        }
                    };
                    set_once(&mut format, parsed, "--format")?;
                    index += 2;
                }
                token if token.starts_with("--") => {
                    return Err(RiskError::Contract(format!(
                        "unknown change risk argument: {token}"
                    )));
                }
                token => {
                    if revspec.replace(token.to_string()).is_some() {
                        return Err(RiskError::Contract(
                            "only one revspec may be supplied".into(),
                        ));
                    }
                    index += 1;
                }
            }
        }
        let revspec = revspec.ok_or_else(|| RiskError::Contract(USAGE.into()))?;
        Ok(Self {
            revspec,
            sample: sample.unwrap_or(DEFAULT_SAMPLE),
            format: format.unwrap_or(Format::Json),
        })
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), RiskError> {
    if slot.replace(value).is_some() {
        Err(RiskError::Contract(format!("duplicate {flag}")))
    } else {
        Ok(())
    }
}

fn run(cli: Cli) -> Result<(Value, Format), RiskError> {
    let repo = resolve_repo_root()?;
    let value = execute(&repo, &cli.revspec, cli.sample)?;
    Ok((value, cli.format))
}

fn print_result(value: &Value, format: Format) {
    match format {
        Format::Json => println!("{}", serde_json::to_string(value).unwrap()),
        Format::Text => println!("{}", render_text(value)),
    }
}

fn resolve_repo_root() -> Result<PathBuf, RiskError> {
    let cwd = std::env::current_dir()
        .map_err(|error| RiskError::HostIo(format!("cannot resolve current directory: {error}")))?;
    let output = hardened_git::command(&cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|error| RiskError::HostIo(format!("cannot launch Git: {error}")))?;
    if !output.status.success() {
        return Err(RiskError::Contract(format!(
            "not a Git repository: {}",
            cwd.display()
        )));
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        return Err(RiskError::Contract(format!(
            "not a Git repository: {}",
            cwd.display()
        )));
    }
    Ok(PathBuf::from(path))
}

// ---------------------------------------------------------------------
// Core scoring
// ---------------------------------------------------------------------

/// (insertions, deletions, repository-relative path)
type FileDiff = (i64, i64, String);

struct DiffShape {
    files_touched: usize,
    insertions: i64,
    deletions: i64,
    lines_changed: i64,
    max_file_share: f64,
    subscore: f64,
}

struct TestAsymmetry {
    source_files_changed: usize,
    test_files_changed: usize,
    asymmetric: bool,
    subscore: f64,
}

struct BugMagnet {
    total_fix_commits: usize,
    files_with_history: usize,
    subscore: f64,
}

struct Churn {
    total_commits: usize,
    subscore: f64,
}

struct Scored {
    score: f64,
    diff: DiffShape,
    test_asymmetry: TestAsymmetry,
    bug_magnet: BugMagnet,
    churn: Churn,
    files: Vec<Value>,
}

/// The two things every scored item (a target revspec or one baseline
/// sample) needs: the exact string handed to `git diff --numstat`, and the
/// tip commit that anchors its time windows.
struct Endpoint {
    range: String,
    tip: String,
}

fn execute(repo: &Path, revspec: &str, sample_requested: u32) -> Result<Value, RiskError> {
    let endpoint = resolve_endpoint(revspec);
    let files = diff_stats(repo, &endpoint.range).unwrap_or_default();
    if files.is_empty() {
        return Ok(build_report(
            revspec,
            None,
            Vec::new(),
            sample_requested,
            0,
            0,
            Some("empty_diff"),
        ));
    }

    let anchor_unix = commit_unix_time(repo, &endpoint.tip)?;
    let exclude = commits_in_range(repo, &endpoint.range)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let target = score_files(repo, &files, anchor_unix, &exclude);

    let mut baseline_scores = Vec::new();
    for (sha, sha_unix) in sample_history(repo, &endpoint.tip, sample_requested) {
        let sample_range = format!("{sha}^..{sha}");
        let sample_files = diff_stats(repo, &sample_range).unwrap_or_default();
        if sample_files.is_empty() {
            continue;
        }
        let sample_exclude: BTreeSet<String> = std::iter::once(sha).collect();
        let sample_scored = score_files(repo, &sample_files, sha_unix, &sample_exclude);
        baseline_scores.push(sample_scored.score);
    }
    let sample_used = baseline_scores.len();
    let percentile = compute_percentile(target.score, &baseline_scores);
    let target_files = target.files.clone();
    Ok(build_report(
        revspec,
        Some(&target),
        target_files,
        sample_requested,
        sample_used,
        percentile,
        None,
    ))
}

/// Splits a revspec into (range-to-diff, tip-to-anchor-on). A "A..B" or
/// "A...B" range is handed to `git diff` as-is (git parses either dot form
/// itself); a bare single commit has no diff of its own, so it is rewritten
/// as "<rev>^..<rev>" to diff it against its first parent.
fn resolve_endpoint(revspec: &str) -> Endpoint {
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
fn tip_token(revspec: &str) -> &str {
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

fn diff_stats(repo: &Path, range: &str) -> Option<Vec<FileDiff>> {
    if let Some(files) = run_git_diff_numstat(repo, range) {
        return Some(files);
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
    None
}

fn run_git_diff_numstat(repo: &Path, range: &str) -> Option<Vec<FileDiff>> {
    let output = hardened_git::command(repo)
        .args(["diff", "--numstat", "--no-renames", range])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(parse_numstat_line)
            .collect(),
    )
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

fn commit_unix_time(repo: &Path, rev: &str) -> Result<i64, RiskError> {
    let output = hardened_git::command(repo)
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
fn commits_in_range(repo: &Path, range: &str) -> Vec<String> {
    let Ok(output) = hardened_git::command(repo)
        .args(["rev-list", range])
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

/// The last `sample` non-merge commits reachable from `start`, each with its
/// own committer-date anchor, fetched in one call so scoring N samples costs
/// N additional git invocations rather than 2N.
fn sample_history(repo: &Path, start: &str, sample: u32) -> Vec<(String, i64)> {
    if sample == 0 {
        return Vec::new();
    }
    let Ok(output) = hardened_git::command(repo)
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

fn score_files(
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
fn is_source_file(path: &str) -> bool {
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
fn is_test_file(path: &str) -> bool {
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

/// One `git log` call covering every touched file at once (mirrors the
/// churn walk in `sentrux_analysis::apply_git_log`): far cheaper than a
/// per-file subprocess. Windowed to the wider of the two signal windows
/// (bug-magnet, 180 days) since churn (90 days) is a subset of it; callers
/// slice the narrower window back out client-side. Commits in `exclude`
/// (the commit set the diff itself was built from) are dropped so a change
/// is never evidence of its own riskiness.
fn file_commit_history(
    repo: &Path,
    files: &[String],
    anchor_unix: i64,
    exclude: &BTreeSet<String>,
) -> BTreeMap<String, Vec<(i64, String)>> {
    let mut history: BTreeMap<String, Vec<(i64, String)>> = files
        .iter()
        .map(|path| (path.clone(), Vec::new()))
        .collect();
    if files.is_empty() {
        return history;
    }
    let since_unix = anchor_unix - BUG_MAGNET_WINDOW_DAYS * 86_400;
    let Ok(output) = hardened_git::command(repo)
        .args([
            "log",
            &format!("--since=@{since_unix}"),
            &format!("--until=@{anchor_unix}"),
            "--format=__CIRISK__%H%x1f%ct%x1f%s",
            "--name-only",
            "--",
        ])
        .args(files.iter())
        .output()
    else {
        return history;
    };
    if !output.status.success() {
        return history;
    }
    let mut current: Option<(String, i64, String)> = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(rest) = line.strip_prefix("__CIRISK__") {
            let mut parts = rest.splitn(3, '\u{1f}');
            let hash = parts.next().unwrap_or("").to_string();
            let timestamp = parts.next().and_then(|value| value.parse::<i64>().ok());
            let subject = parts.next().unwrap_or("").to_string();
            current = timestamp.map(|timestamp| (hash, timestamp, subject));
            continue;
        }
        let path = line.trim();
        if path.is_empty() {
            continue;
        }
        let Some((hash, timestamp, subject)) = &current else {
            continue;
        };
        if exclude.contains(hash) {
            continue;
        }
        if let Some(entries) = history.get_mut(&normalize_path(path)) {
            entries.push((*timestamp, subject.clone()));
        }
    }
    history
}

/// Matches `/fix|修复|修正/i` without a regex dependency: "fix" has no case
/// variants worth folding beyond ASCII lowercasing, and the two Chinese
/// markers have no case at all.
fn looks_like_fix_subject(subject: &str) -> bool {
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

fn combine(
    diff: &DiffShape,
    asymmetry: &TestAsymmetry,
    bug_magnet: &BugMagnet,
    churn: &Churn,
) -> f64 {
    WEIGHT_DIFF_SHAPE * diff.subscore
        + WEIGHT_TEST_ASYMMETRY * asymmetry.subscore
        + WEIGHT_BUG_MAGNET * bug_magnet.subscore
        + WEIGHT_CHURN * churn.subscore
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

/// Percentage of the baseline sample scoring at or below the target: 100
/// means the target is at least as risky as every sampled commit, 0 means
/// every sampled commit scored higher. An empty sample (no history to
/// compare against, e.g. a brand-new repository) reports 0 rather than an
/// undefined value — a new gate should fail open when it has no evidence,
/// not fail closed.
fn compute_percentile(target_score: f64, baseline: &[f64]) -> u32 {
    if baseline.is_empty() {
        return 0;
    }
    let at_or_below = baseline
        .iter()
        .filter(|&&score| score <= target_score)
        .count();
    ((at_or_below as f64 / baseline.len() as f64) * 100.0).round() as u32
}

fn level_for_percentile(percentile: u32) -> &'static str {
    if percentile >= 90 {
        "high"
    } else if percentile >= 60 {
        "medium"
    } else {
        "low"
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn build_report(
    revspec: &str,
    scored: Option<&Scored>,
    files_value: Vec<Value>,
    sample_requested: u32,
    sample_used: usize,
    percentile: u32,
    warning: Option<&str>,
) -> Value {
    let score = scored.map_or(0, |scored| scored.score.round() as i64);
    let level = level_for_percentile(percentile);
    let signals = match scored {
        Some(scored) => json!({
            "diff": {
                "filesTouched": scored.diff.files_touched,
                "insertions": scored.diff.insertions,
                "deletions": scored.diff.deletions,
                "linesChanged": scored.diff.lines_changed,
                "maxFileShare": round2(scored.diff.max_file_share),
                "subscore": round2(scored.diff.subscore),
            },
            "testAsymmetry": {
                "sourceFilesChanged": scored.test_asymmetry.source_files_changed,
                "testFilesChanged": scored.test_asymmetry.test_files_changed,
                "asymmetric": scored.test_asymmetry.asymmetric,
                "subscore": round2(scored.test_asymmetry.subscore),
            },
            "bugMagnet": {
                "windowDays": BUG_MAGNET_WINDOW_DAYS,
                "totalFixCommits": scored.bug_magnet.total_fix_commits,
                "filesWithHistory": scored.bug_magnet.files_with_history,
                "subscore": round2(scored.bug_magnet.subscore),
            },
            "churn": {
                "windowDays": CHURN_WINDOW_DAYS,
                "totalCommits": scored.churn.total_commits,
                "subscore": round2(scored.churn.subscore),
            },
            "weights": {
                "diffShape": WEIGHT_DIFF_SHAPE,
                "testAsymmetry": WEIGHT_TEST_ASYMMETRY,
                "bugMagnet": WEIGHT_BUG_MAGNET,
                "churn": WEIGHT_CHURN,
            },
            "sampleRequested": sample_requested,
            "sampleUsed": sample_used,
        }),
        None => json!({
            "diff": {
                "filesTouched": 0, "insertions": 0, "deletions": 0, "linesChanged": 0,
                "maxFileShare": 0.0, "subscore": 0.0,
            },
            "testAsymmetry": {
                "sourceFilesChanged": 0, "testFilesChanged": 0, "asymmetric": false, "subscore": 0.0,
            },
            "bugMagnet": {
                "windowDays": BUG_MAGNET_WINDOW_DAYS, "totalFixCommits": 0, "filesWithHistory": 0, "subscore": 0.0,
            },
            "churn": {
                "windowDays": CHURN_WINDOW_DAYS, "totalCommits": 0, "subscore": 0.0,
            },
            "weights": {
                "diffShape": WEIGHT_DIFF_SHAPE, "testAsymmetry": WEIGHT_TEST_ASYMMETRY,
                "bugMagnet": WEIGHT_BUG_MAGNET, "churn": WEIGHT_CHURN,
            },
            "sampleRequested": sample_requested,
            "sampleUsed": sample_used,
        }),
    };
    let mut result = json!({
        "schema": "code-intel-change-risk.v1",
        "revspec": revspec,
        "score": score,
        "risk_percentile": percentile,
        "level": level,
        "signals": signals,
        "files": files_value,
    });
    if let Some(warning) = warning {
        result["warning"] = json!(warning);
    }
    result
}

/// Derived strictly from the same JSON `Value` the JSON output serializes —
/// never recomputed — so `--format text` cannot drift from `--format json`.
fn render_text(value: &Value) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "revspec: {}",
        value["revspec"].as_str().unwrap_or("")
    ));
    if let Some(warning) = value.get("warning").and_then(Value::as_str) {
        lines.push(format!("warning: {warning}"));
    }
    lines.push(format!(
        "score: {} (percentile: {}, level: {})",
        value["score"],
        value["risk_percentile"],
        value["level"].as_str().unwrap_or("")
    ));
    let signals = &value["signals"];
    lines.push(format!(
        "diff: {} file(s), +{}/-{}, maxFileShare={}",
        signals["diff"]["filesTouched"],
        signals["diff"]["insertions"],
        signals["diff"]["deletions"],
        signals["diff"]["maxFileShare"]
    ));
    lines.push(format!(
        "testAsymmetry: source={} test={} asymmetric={}",
        signals["testAsymmetry"]["sourceFilesChanged"],
        signals["testAsymmetry"]["testFilesChanged"],
        signals["testAsymmetry"]["asymmetric"]
    ));
    lines.push(format!(
        "bugMagnet (window={}d): totalFixCommits={} filesWithHistory={}",
        signals["bugMagnet"]["windowDays"],
        signals["bugMagnet"]["totalFixCommits"],
        signals["bugMagnet"]["filesWithHistory"]
    ));
    lines.push(format!(
        "churn (window={}d): totalCommits={}",
        signals["churn"]["windowDays"], signals["churn"]["totalCommits"]
    ));
    lines.push(format!(
        "sample: used {} of {} requested",
        signals["sampleUsed"], signals["sampleRequested"]
    ));
    let files = value["files"].as_array().cloned().unwrap_or_default();
    lines.push(format!("files: {}", files.len()));
    for file in &files {
        let marker = if file["isTestFile"].as_bool().unwrap_or(false) {
            " [test]"
        } else {
            ""
        };
        lines.push(format!(
            "  {} +{}/-{}{marker}",
            file["path"].as_str().unwrap_or(""),
            file["insertions"],
            file["deletions"]
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn init_repo(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repo = std::env::temp_dir().join(format!(
            "code-intel-change-risk-{name}-{nonce}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&repo).unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.name", "Change Risk Test"],
            vec!["config", "user.email", "change-risk@example.invalid"],
            vec!["config", "commit.gpgsign", "false"],
        ] {
            assert!(Command::new("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success());
        }
        repo
    }

    fn write_file(repo: &Path, relative: &str, contents: &str) {
        let path = repo.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    fn commit(repo: &Path, message: &str) {
        assert!(Command::new("git")
            .args(["add", "."])
            .current_dir(repo)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["commit", "--quiet", "-m", message])
            .current_dir(repo)
            .status()
            .unwrap()
            .success());
    }

    #[test]
    fn weights_sum_to_one_hundred() {
        assert_eq!(
            WEIGHT_DIFF_SHAPE + WEIGHT_TEST_ASYMMETRY + WEIGHT_BUG_MAGNET + WEIGHT_CHURN,
            100.0
        );
    }

    #[test]
    fn is_source_file_matches_the_crates_src_glob() {
        assert!(is_source_file("crates/code-intel-cli/src/main.rs"));
        assert!(is_source_file("crates/code-intel-cli/src/sub/mod.rs"));
        assert!(!is_source_file("crates/code-intel-cli/Cargo.toml"));
        assert!(!is_source_file("crates/code-intel-cli/tests/it.rs"));
        assert!(!is_source_file("README.md"));
    }

    #[test]
    fn is_test_file_matches_the_documented_heuristics() {
        assert!(is_test_file("crates/code-intel-cli/tests/it.rs"));
        assert!(is_test_file("crates/code-intel-cli/src/foo_test.rs"));
        assert!(is_test_file("crates/code-intel-cli/src/test_foo.rs"));
        assert!(!is_test_file("crates/code-intel-cli/src/main.rs"));
    }

    #[test]
    fn looks_like_fix_subject_matches_english_and_chinese_markers() {
        assert!(looks_like_fix_subject("fix(gate): correct off-by-one"));
        assert!(looks_like_fix_subject("Fix regression in parser"));
        assert!(looks_like_fix_subject("修复解析器越界问题"));
        assert!(looks_like_fix_subject("修正边界条件"));
        assert!(!looks_like_fix_subject("feat: add change risk subcommand"));
    }

    #[test]
    fn tip_token_extracts_the_range_head() {
        assert_eq!(tip_token("origin/main..HEAD"), "HEAD");
        assert_eq!(tip_token("abc123...def456"), "def456");
        assert_eq!(tip_token("origin/main.."), "HEAD");
        assert_eq!(tip_token("abc123"), "abc123");
    }

    #[test]
    fn empty_diff_reports_a_warning_without_erroring() {
        let repo = init_repo("empty-diff");
        write_file(&repo, "README.md", "hello\n");
        commit(&repo, "chore: seed repository");

        let identical = execute(&repo, "HEAD..HEAD", 5).expect("identical range never errors");
        assert_eq!(identical["warning"], "empty_diff");
        assert_eq!(identical["score"], 0);
        assert_eq!(identical["risk_percentile"], 0);
        assert_eq!(identical["files"].as_array().unwrap().len(), 0);

        let bogus = execute(&repo, "definitely-not-a-real-branch..HEAD", 5)
            .expect("an unresolvable revspec degrades to a warning, not an error");
        assert_eq!(bogus["warning"], "empty_diff");

        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn scoring_is_deterministic_for_the_same_revspec() {
        let repo = init_repo("determinism");
        write_file(&repo, "README.md", "hello\n");
        write_file(&repo, "crates/demo/src/lib.rs", "pub fn demo() {}\n");
        commit(&repo, "feat: seed demo crate");

        write_file(
            &repo,
            "crates/demo/src/lib.rs",
            "pub fn demo() { println!(\"hi\"); }\n",
        );
        commit(&repo, "fix: correct demo output");

        let first = execute(&repo, "HEAD~1..HEAD", 10).expect("scoring succeeds");
        let second = execute(&repo, "HEAD~1..HEAD", 10).expect("scoring succeeds");
        assert_eq!(
            first, second,
            "identical input must produce an identical report"
        );
        assert!(first.get("warning").is_none());
        assert_eq!(first["signals"]["testAsymmetry"]["asymmetric"], true);

        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn root_commit_diffs_against_the_empty_tree() {
        let repo = init_repo("root-commit");
        write_file(&repo, "crates/demo/src/lib.rs", "pub fn demo() {}\n");
        commit(&repo, "feat: initial commit");

        let result = execute(&repo, "HEAD", 5).expect("root commit scoring never errors");
        assert!(
            result.get("warning").is_none(),
            "root commit has a real diff: {result}"
        );
        assert_eq!(result["files"].as_array().unwrap().len(), 1);

        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn a_commit_inside_the_scored_range_does_not_count_toward_its_own_files_history() {
        let repo = init_repo("self-reference");
        write_file(&repo, "crates/demo/src/lib.rs", "fn a() {}\n");
        commit(&repo, "feat: seed demo crate");

        // An internal commit inside the range about to be scored: a real fix
        // touching the very file the range's own diff will report as
        // changed. Without the exclusion in `file_commit_history`, this
        // would count toward that file's own bug-magnet tally.
        write_file(
            &repo,
            "crates/demo/src/lib.rs",
            "fn a() { /* patched */ }\n",
        );
        commit(&repo, "fix: patch demo crash");

        write_file(
            &repo,
            "crates/demo/src/lib.rs",
            "fn a() { /* patched */ }\nfn b() {}\n",
        );
        commit(&repo, "feat: extend demo crate");

        // Score the whole three-commit range in one shot, so both the fix
        // commit and the tip commit are "inside" the diff being scored.
        let result = execute(&repo, "HEAD~2..HEAD", 0).expect("scoring succeeds");
        assert!(result.get("warning").is_none());
        let files = result["files"].as_array().expect("files array");
        let lib_file = files
            .iter()
            .find(|file| file["path"] == "crates/demo/src/lib.rs")
            .expect("lib.rs is in the diff");
        assert_eq!(
            lib_file["bugFixCommits180d"], 0,
            "a fix commit inside the scored range must not count toward its own file's bug-magnet tally: {result}"
        );
        assert_eq!(result["signals"]["bugMagnet"]["totalFixCommits"], 0);

        std::fs::remove_dir_all(&repo).ok();
    }
}
