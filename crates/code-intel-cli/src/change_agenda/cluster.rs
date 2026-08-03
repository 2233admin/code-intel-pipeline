//! Partitioning the changed files into review units by co-change edges.
//! Union-find over the kept edges: two files land in the same unit when
//! history commits them together often enough, transitively. Files no edge
//! reaches stay singletons — that is a real answer ("nothing in this change
//! is coupled to it"), not a failure to cluster.

use std::collections::BTreeMap;

use super::cochange::Edge;

/// Groups `paths` under the equivalence closure of `edges`.
///
/// Output order is deterministic and derived only from the paths: each
/// group's members are sorted, and the groups themselves are ordered by
/// their first member. Callers re-rank by score afterwards; this ordering
/// exists so the partition itself never depends on hash-map iteration or
/// on the order edges happened to arrive in.
pub(super) fn group(paths: &[String], edges: &[Edge]) -> Vec<Vec<String>> {
    let index: BTreeMap<&str, usize> = paths
        .iter()
        .enumerate()
        .map(|(position, path)| (path.as_str(), position))
        .collect();
    let mut parent: Vec<usize> = (0..paths.len()).collect();
    for edge in edges {
        let (Some(&left), Some(&right)) = (
            index.get(edge.left.as_str()),
            index.get(edge.right.as_str()),
        ) else {
            // An edge naming a path outside the changed set cannot exist:
            // edges are built from the same `git log -- <changed files>`
            // walk. Skipping rather than panicking keeps a future caller
            // that widens the walk from turning a widened input into a
            // crash.
            continue;
        };
        union(&mut parent, left, right);
    }
    let mut groups: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (position, path) in paths.iter().enumerate() {
        groups
            .entry(find(&mut parent, position))
            .or_default()
            .push(path.clone());
    }
    let mut groups: Vec<Vec<String>> = groups
        .into_values()
        .map(|mut members| {
            members.sort();
            members
        })
        .collect();
    groups.sort_by(|first, second| first.first().cmp(&second.first()));
    groups
}

fn find(parent: &mut [usize], mut node: usize) -> usize {
    while parent[node] != node {
        // Path halving: keeps the walk near-flat without a second pass.
        parent[node] = parent[parent[node]];
        node = parent[node];
    }
    node
}

fn union(parent: &mut [usize], left: usize, right: usize) {
    let left = find(parent, left);
    let right = find(parent, right);
    if left == right {
        return;
    }
    // Always attach the higher index under the lower one: union order then
    // has no effect on the resulting roots, only on how deep the tree gets
    // before halving flattens it.
    let (root, child) = if left < right {
        (left, right)
    } else {
        (right, left)
    };
    parent[child] = root;
}
