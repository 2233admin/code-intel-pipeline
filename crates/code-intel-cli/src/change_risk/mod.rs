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
//!
//! ## Layout
//!
//! This module was split out of a single ~1150-line file into focused
//! submodules: [`git`] (all `git` subprocess plumbing), [`signals`] (the
//! four per-signal computations), [`scoring`] (weighting, percentile,
//! level), and [`render`] (JSON report assembly + `--format text`). This
//! file keeps the CLI entry point and the top-level `execute` orchestration
//! that ties the submodules together, plus the shared types and the
//! documented scoring constants every submodule draws from.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

pub(crate) mod git;
mod render;
mod scoring;
mod signals;
#[cfg(test)]
mod tests;

const USAGE: &str =
    "usage: change risk <revspec> [--repo <path>] [--sample <N>] [--format json|text]";

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

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    // Leaf adapter: parse + compute + render. Controllers wrap execute_request
    // for typed authority results; they must not be imported here or the
    // workspace-advisory layer forms an import cycle with this module.
    match ChangeRiskRequest::parse(raw).and_then(execute_request) {
        Ok(result) => {
            print_result(result.value(), result.format());
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
pub(crate) enum RiskError {
    Contract(String),
    HostIo(String),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Format {
    Json,
    Text,
}

pub(crate) struct ChangeRiskRequest {
    revspec: String,
    sample: u32,
    format: Format,
    /// Explicit repo root, from `--repo <path>` (issue #114). `None` means
    /// "keep resolving from the current directory" — the pre-existing
    /// default behavior, unchanged when the flag is absent.
    repo: Option<PathBuf>,
}

impl ChangeRiskRequest {
    pub(crate) fn parse(raw: &[String]) -> Result<Self, RiskError> {
        if raw.first().map(String::as_str) != Some("risk") {
            return Err(RiskError::Contract(USAGE.into()));
        }
        let mut revspec: Option<String> = None;
        let mut sample: Option<u32> = None;
        let mut format: Option<Format> = None;
        let mut repo: Option<PathBuf> = None;
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
                "--repo" => {
                    let value = raw
                        .get(index + 1)
                        .filter(|value| !value.is_empty() && !value.starts_with("--"))
                        .ok_or_else(|| RiskError::Contract("--repo requires one value".into()))?;
                    set_once(&mut repo, PathBuf::from(value), "--repo")?;
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
            repo,
        })
    }

    pub(crate) fn format(&self) -> Format {
        self.format
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), RiskError> {
    if slot.replace(value).is_some() {
        Err(RiskError::Contract(format!("duplicate {flag}")))
    } else {
        Ok(())
    }
}

pub(crate) struct ChangeRiskResult {
    value: Value,
    format: Format,
    repo_root: PathBuf,
    revspec: String,
}

impl ChangeRiskResult {
    pub(crate) fn value(&self) -> &Value {
        &self.value
    }

    pub(crate) fn format(&self) -> Format {
        self.format
    }

    pub(crate) fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    pub(crate) fn revspec(&self) -> &str {
        &self.revspec
    }
}

pub(crate) fn execute_request(cli: ChangeRiskRequest) -> Result<ChangeRiskResult, RiskError> {
    let repo = match &cli.repo {
        Some(path) => git::resolve_repo_root_from(path)?,
        None => git::resolve_repo_root()?,
    };
    let value = execute(&repo, &cli.revspec, cli.sample)?;
    Ok(ChangeRiskResult {
        value,
        format: cli.format,
        repo_root: repo,
        revspec: cli.revspec,
    })
}

fn print_result(value: &Value, format: Format) {
    match format {
        Format::Json => println!("{}", serde_json::to_string(value).unwrap()),
        Format::Text => println!("{}", render::render_text(value)),
    }
}

// ---------------------------------------------------------------------
// Core scoring
// ---------------------------------------------------------------------

/// (insertions, deletions, repository-relative path)
pub(crate) type FileDiff = (i64, i64, String);

/// Repository-relative path -> the (committer date, subject) of every commit
/// touching it inside the bug-magnet window.
pub(crate) type FileHistory = BTreeMap<String, Vec<(i64, String)>>;

#[derive(Default)]
struct DiffShape {
    files_touched: usize,
    insertions: i64,
    deletions: i64,
    lines_changed: i64,
    max_file_share: f64,
    subscore: f64,
}

#[derive(Default)]
struct TestAsymmetry {
    source_files_changed: usize,
    test_files_changed: usize,
    asymmetric: bool,
    subscore: f64,
}

#[derive(Default)]
struct BugMagnet {
    total_fix_commits: usize,
    files_with_history: usize,
    subscore: f64,
}

#[derive(Default)]
struct Churn {
    total_commits: usize,
    subscore: f64,
}

/// `#[derive(Default)]` backs `render::build_report`'s no-score (warning)
/// path: a zero-valued `Scored` defines the empty-report signal shape once,
/// instead of a second hand-written all-zeros JSON literal that could drift
/// from the real one.
#[derive(Default)]
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
pub(crate) struct Endpoint {
    pub(crate) range: String,
    pub(crate) tip: String,
}

/// One scored subset of a change, for consumers that need a score over
/// *part* of a diff on the same scale `change risk` reports for the whole
/// of it.
///
/// `change agenda` (issue #150) scores every review unit through this, so a
/// unit's number and the PR gate's number come out of one scorer and cannot
/// disagree. Two independently-derived numbers over one tree is exactly the
/// defect issue #148 C2 records (277 vs 315 files, unreconcilable); a
/// second risk formula for units would have reproduced it.
pub(crate) struct SubsetScore {
    pub(crate) score: f64,
    pub(crate) signals: Value,
    pub(crate) files: Vec<Value>,
}

/// Scores `files` — a subset of an already-walked diff — against a history
/// map the caller fetched once for the whole diff. No git, no I/O.
pub(crate) fn score_subset(
    files: &[FileDiff],
    history: &FileHistory,
    anchor_unix: i64,
) -> SubsetScore {
    let scored = signals::score_files_with_history(files, history, anchor_unix);
    SubsetScore {
        score: scored.score,
        signals: render::signals_value(&scored),
        files: scored.files,
    }
}

fn execute(repo: &Path, revspec: &str, sample_requested: u32) -> Result<Value, RiskError> {
    let endpoint = git::resolve_endpoint(revspec);
    let files = git::diff_stats(repo, &endpoint.range)?;
    if files.is_empty() {
        return Ok(render::build_report(
            repo,
            revspec,
            None,
            Vec::new(),
            sample_requested,
            0,
            0,
            Some("empty_diff"),
        ));
    }

    let anchor_unix = git::commit_unix_time(repo, &endpoint.tip)?;
    let exclude = git::commits_in_range(repo, &endpoint.range)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let target = signals::score_files(repo, &files, anchor_unix, &exclude);

    let mut baseline_scores = Vec::new();
    for (sha, sha_unix) in git::sample_history(repo, &endpoint.tip, sample_requested) {
        // A commit inside the range being scored (the tip itself, or any
        // ancestor up to the range base) must never also serve as a
        // baseline sample: it is part of the very thing being evaluated,
        // and — being a strict subset of the target's own diff — it is
        // near-guaranteed to score at or below the (larger, aggregated)
        // target, which mechanically inflates `risk_percentile` regardless
        // of whether the change is actually riskier than genuine history.
        if exclude.contains(&sha) {
            continue;
        }
        let sample_range = format!("{sha}^..{sha}");
        let sample_files = git::diff_stats(repo, &sample_range).unwrap_or_default();
        if sample_files.is_empty() {
            continue;
        }
        let sample_exclude: BTreeSet<String> = std::iter::once(sha).collect();
        let sample_scored = signals::score_files(repo, &sample_files, sha_unix, &sample_exclude);
        baseline_scores.push(sample_scored.score);
    }
    let sample_used = baseline_scores.len();
    let percentile = scoring::compute_percentile(target.score, &baseline_scores);
    let target_files = target.files.clone();
    Ok(render::build_report(
        repo,
        revspec,
        Some(&target),
        target_files,
        sample_requested,
        sample_used,
        percentile,
        None,
    ))
}
