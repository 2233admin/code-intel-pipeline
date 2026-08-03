//! `code-intel change agenda <revspec>` — a deterministic, git-only review
//! agenda for a commit or range: the changed files partitioned into review
//! units by their co-change history, each unit scored by the same scorer
//! `change risk` scores the whole change with, ranked worst first.
//!
//! ## Why this exists
//!
//! A gate that reports a number and not the objects behind it hands the
//! ranking work back to whoever reads it — for an agent, that means paying
//! to re-derive an order the pipeline already had the evidence to produce
//! (issue #150; issue #148 C1 is the same complaint against the monolith
//! gate, which reports `God files: 33 -> 33` and zero filenames).
//!
//! ## Boundary
//!
//! Git only: no index, no committed run, no network, no LLM — the same
//! contract as `change risk`, and for the same reason (it has to answer on
//! a PR before any of those exist). Everything requiring committed
//! evidence — test selection, structural-rule hits — is reported as
//! explicitly unavailable with the command that produces it, never
//! approximated. Folding those in behind a flag would make one command
//! sometimes admissible and sometimes not, which is the property the
//! `edit impact` route comment already records as the one callers cannot
//! afford to guess at.
//!
//! ## Determinism
//!
//! Every time window is anchored to the scored commit's own committer date
//! (inherited from `change risk`), every ordering falls back to a path or
//! hash comparison, and nothing reads the wall clock. The same revspec on
//! the same repository state produces the same bytes forever.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::change_risk::{self, git, FileDiff, FileHistory, Format, RiskError};

mod cluster;
mod cochange;
mod render;
#[cfg(test)]
mod tests;

const USAGE: &str =
    "usage: change agenda <revspec> [--repo <path>] [--format json|text] [--min-cochange <N>]";

/// Trailing window for the co-change walk. Four times the churn window and
/// twice the bug-magnet one: coupling is a structural property that moves
/// slowly, and a 90-day view of a module nobody touched last quarter
/// reports "uncoupled" for files that have shipped together for years.
const COCHANGE_WINDOW_DAYS: i64 = 365;

/// How many shared commits before two changed files are treated as coupled.
/// Two is one coincidence away from noise — a pair of files can land in the
/// same commit once for no structural reason at all. Three is the smallest
/// count that needs a habit to produce.
const DEFAULT_MIN_COCHANGE: usize = 3;

/// Commit hashes retained per edge as traceable evidence. Enough to open
/// and check the claim; the full count stays in `coCommits`, and
/// `commitsTruncated` marks when the list is a sample rather than all of it.
const EDGE_EVIDENCE_LIMIT: usize = 3;

/// A commit touching more of the changed files than this is a sweep — a
/// format pass, a mass rename, a license-header update — and couples every
/// file it touches with every other one. One such commit is enough to fuse
/// an entire change into a single unit that says nothing, so they are
/// dropped from the coupling evidence and counted in the report.
const COCHANGE_COMMIT_FANOUT_CAP: usize = 50;

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    // Leaf adapter: parse + compute + render, mirroring `change risk`.
    // Controllers wrap `execute_request` for typed authority results and
    // must not be imported here, or the workspace-advisory layer forms an
    // import cycle with this module.
    match ChangeAgendaRequest::parse(raw).and_then(execute_request) {
        Ok(result) => {
            match result.format {
                Format::Json => println!("{}", serde_json::to_string(&result.value).unwrap()),
                Format::Text => println!("{}", render::render_text(&result.value)),
            }
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
pub(crate) struct ChangeAgendaRequest {
    revspec: String,
    format: Format,
    min_cochange: usize,
    /// Explicit repo root from `--repo <path>`; `None` resolves from the
    /// current directory, matching `change risk`.
    repo: Option<PathBuf>,
}

impl ChangeAgendaRequest {
    pub(crate) fn parse(raw: &[String]) -> Result<Self, RiskError> {
        if raw.first().map(String::as_str) != Some("agenda") {
            return Err(RiskError::Contract(USAGE.into()));
        }
        let mut revspec: Option<String> = None;
        let mut format: Option<Format> = None;
        let mut min_cochange: Option<usize> = None;
        let mut repo: Option<PathBuf> = None;
        let mut index = 1;
        while index < raw.len() {
            let token = raw[index].as_str();
            match token {
                "--format" => {
                    let value = value_for(raw, index, "--format")?;
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
                "--min-cochange" => {
                    let value = value_for(raw, index, "--min-cochange")?;
                    let parsed: usize = value.parse().map_err(|_| {
                        RiskError::Contract(format!(
                            "--min-cochange must be a non-negative integer: {value}"
                        ))
                    })?;
                    if parsed < 2 {
                        return Err(RiskError::Contract(
                            "--min-cochange must be at least 2: a pair needs two commits before it is a pattern".into(),
                        ));
                    }
                    set_once(&mut min_cochange, parsed, "--min-cochange")?;
                    index += 2;
                }
                "--repo" => {
                    let value = value_for(raw, index, "--repo")?;
                    set_once(&mut repo, PathBuf::from(value), "--repo")?;
                    index += 2;
                }
                token if token.starts_with("--") => {
                    return Err(RiskError::Contract(format!(
                        "unknown change agenda argument: {token}"
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
        Ok(Self {
            revspec: revspec.ok_or_else(|| RiskError::Contract(USAGE.into()))?,
            format: format.unwrap_or(Format::Json),
            min_cochange: min_cochange.unwrap_or(DEFAULT_MIN_COCHANGE),
            repo,
        })
    }
}

fn value_for<'a>(raw: &'a [String], index: usize, flag: &str) -> Result<&'a String, RiskError> {
    raw.get(index + 1)
        .filter(|value| !value.is_empty() && !value.starts_with("--"))
        .ok_or_else(|| RiskError::Contract(format!("{flag} requires one value")))
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), RiskError> {
    if slot.replace(value).is_some() {
        Err(RiskError::Contract(format!("duplicate {flag}")))
    } else {
        Ok(())
    }
}

pub(crate) struct ChangeAgendaResult {
    value: Value,
    format: Format,
    repo_root: PathBuf,
    revspec: String,
}

impl ChangeAgendaResult {
    pub(crate) fn value(&self) -> &Value {
        &self.value
    }

    pub(crate) fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    pub(crate) fn revspec(&self) -> &str {
        &self.revspec
    }
}

pub(crate) fn execute_request(cli: ChangeAgendaRequest) -> Result<ChangeAgendaResult, RiskError> {
    let repo = match &cli.repo {
        Some(path) => git::resolve_repo_root_from(path)?,
        None => git::resolve_repo_root()?,
    };
    let value = execute(&repo, &cli.revspec, cli.min_cochange)?;
    Ok(ChangeAgendaResult {
        value,
        format: cli.format,
        repo_root: repo,
        revspec: cli.revspec,
    })
}

/// One review unit: the changed files history commits together, the edges
/// that joined them, and the same-scorer score for that subset.
struct Unit {
    members: Vec<String>,
    score: f64,
    signals: Value,
    file_rows: Vec<Value>,
    edges: Vec<cochange::Edge>,
}

fn execute(repo: &Path, revspec: &str, min_cochange: usize) -> Result<Value, RiskError> {
    let endpoint = git::resolve_endpoint(revspec);
    let files = git::diff_stats(repo, &endpoint.range)?;
    if files.is_empty() {
        return Ok(render::build_report(render::Report {
            repo,
            revspec,
            endpoint: &endpoint,
            anchor_unix: 0,
            changed_files: 0,
            units: &[],
            observed: &cochange::Observed::default(),
            min_cochange,
            warning: Some("empty_diff"),
        }));
    }

    let anchor_unix = git::commit_unix_time(repo, &endpoint.tip)?;
    let exclude: BTreeSet<String> = git::commits_in_range(repo, &endpoint.range)
        .into_iter()
        .collect();
    let mut paths: Vec<String> = files.iter().map(|(_, _, path)| path.clone()).collect();
    paths.sort();

    // One history walk for the whole diff; every unit scores against this
    // same map rather than paying for a `git log` apiece.
    let history = git::file_commit_history(repo, &paths, anchor_unix, &exclude);
    let commits =
        git::cochanging_commits(repo, &paths, anchor_unix, COCHANGE_WINDOW_DAYS, &exclude);
    let (edges, observed) = cochange::build_edges(&commits, min_cochange);

    let mut units: Vec<Unit> = cluster::group(&paths, &edges)
        .into_iter()
        .map(|members| build_unit(members, &files, &edges, &history, anchor_unix))
        .collect();
    // Worst first. `total_cmp` rather than `partial_cmp().unwrap()`: the
    // scores are finite by construction, and this states that rather than
    // asserting it at runtime. The member-path tiebreak keeps equal-scoring
    // units in a fixed order.
    units.sort_by(|first, second| {
        second
            .score
            .total_cmp(&first.score)
            .then_with(|| first.members.first().cmp(&second.members.first()))
    });

    Ok(render::build_report(render::Report {
        repo,
        revspec,
        endpoint: &endpoint,
        anchor_unix,
        changed_files: files.len(),
        units: &units,
        observed: &observed,
        min_cochange,
        warning: None,
    }))
}

fn build_unit(
    members: Vec<String>,
    files: &[FileDiff],
    edges: &[cochange::Edge],
    history: &FileHistory,
    anchor_unix: i64,
) -> Unit {
    let member_set: BTreeSet<&str> = members.iter().map(String::as_str).collect();
    let mut unit_files: Vec<FileDiff> = files
        .iter()
        .filter(|(_, _, path)| member_set.contains(path.as_str()))
        .cloned()
        .collect();
    unit_files.sort_by(|first, second| first.2.cmp(&second.2));
    let unit_edges: Vec<cochange::Edge> = edges
        .iter()
        .filter(|edge| {
            member_set.contains(edge.left.as_str()) && member_set.contains(edge.right.as_str())
        })
        .cloned()
        .collect();
    let scored = change_risk::score_subset(&unit_files, history, anchor_unix);
    Unit {
        members,
        score: scored.score,
        signals: scored.signals,
        file_rows: scored.files,
        edges: unit_edges,
    }
}
