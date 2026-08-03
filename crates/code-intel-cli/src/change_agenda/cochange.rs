//! Turning a co-change log walk into weighted edges between changed files.
//! Pure: everything here is a function of the commits `change_risk::git`
//! already returned, so the clustering it feeds is unit-testable without a
//! repository.

use std::collections::BTreeMap;

use crate::change_risk::git::LogCommit;

use super::{COCHANGE_COMMIT_FANOUT_CAP, EDGE_EVIDENCE_LIMIT};

/// Two changed files that history commits together, with the evidence that
/// says so. `left` is always the lexicographically smaller path, so a pair
/// has exactly one key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Edge {
    pub(super) left: String,
    pub(super) right: String,
    pub(super) co_commits: usize,
    /// The most recent commits backing this edge, newest first. Bounded by
    /// [`EDGE_EVIDENCE_LIMIT`]; `commits_truncated` says when more exist.
    pub(super) commits: Vec<String>,
    pub(super) commits_truncated: bool,
}

/// What the walk saw, whether or not it produced edges. Reported verbatim
/// so "no units clustered" can be told apart from "no history to cluster
/// on" — a silently empty coupling signal reads as "these files are
/// independent", which is a different claim entirely.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Observed {
    pub(super) commits_walked: usize,
    pub(super) pairs_observed: usize,
    pub(super) edges_kept: usize,
    /// Commits dropped for touching more of the changed files than
    /// [`COCHANGE_COMMIT_FANOUT_CAP`] allows.
    pub(super) wide_commits_skipped: usize,
}

/// Counts every co-occurrence pair across `commits`, keeps the pairs seen at
/// least `min_commits` times, and returns them ranked strongest first.
///
/// Commits touching more than [`COCHANGE_COMMIT_FANOUT_CAP`] of the changed
/// files are dropped rather than counted: a format sweep, a mass rename, or
/// a license-header pass couples every file with every other file, and one
/// such commit would fuse the whole change into a single unit that says
/// nothing. They are counted into `wide_commits_skipped` rather than
/// silently discarded.
pub(super) fn build_edges(commits: &[LogCommit], min_commits: usize) -> (Vec<Edge>, Observed) {
    let mut observed = Observed {
        commits_walked: commits.len(),
        ..Observed::default()
    };
    let mut pairs: BTreeMap<(&str, &str), Vec<(i64, &str)>> = BTreeMap::new();
    for commit in commits {
        if commit.paths.len() > COCHANGE_COMMIT_FANOUT_CAP {
            observed.wide_commits_skipped += 1;
            continue;
        }
        for (index, left) in commit.paths.iter().enumerate() {
            for right in &commit.paths[index + 1..] {
                pairs
                    .entry((left.as_str(), right.as_str()))
                    .or_default()
                    .push((commit.timestamp, commit.hash.as_str()));
            }
        }
    }
    observed.pairs_observed = pairs.len();
    let mut edges: Vec<Edge> = pairs
        .into_iter()
        .filter_map(|((left, right), mut hits)| {
            let co_commits = hits.len();
            if co_commits < min_commits {
                return None;
            }
            // Newest evidence first — that is the commit a reviewer will
            // actually open — with the hash as tiebreak so two commits
            // sharing a committer second still order deterministically.
            hits.sort_by(|first, second| {
                second.0.cmp(&first.0).then_with(|| first.1.cmp(second.1))
            });
            Some(Edge {
                left: left.to_string(),
                right: right.to_string(),
                co_commits,
                commits_truncated: hits.len() > EDGE_EVIDENCE_LIMIT,
                commits: hits
                    .into_iter()
                    .take(EDGE_EVIDENCE_LIMIT)
                    .map(|(_, hash)| hash.to_string())
                    .collect(),
            })
        })
        .collect();
    edges.sort_by(|first, second| {
        second
            .co_commits
            .cmp(&first.co_commits)
            .then_with(|| first.left.cmp(&second.left))
            .then_with(|| first.right.cmp(&second.right))
    });
    observed.edges_kept = edges.len();
    (edges, observed)
}
