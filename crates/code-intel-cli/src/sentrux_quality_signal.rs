//! Deterministic Quality Signal computation kernel (issue #385, child of
//! #285). Replaces `sentrux_gate.rs`'s old `10000 - penalty` proxy score,
//! which was semantically incompatible with the upstream Sentrux Quality
//! Signal contract (https://sentrux.dev/docs/quality-signal/) and exposed no
//! root causes at all.
//!
//! This module is the pure math kernel: normalization, geometric-mean
//! aggregation, and the graph algorithms (Newman modularity Q, SCC-based
//! cycle/depth) that turn already-extracted facts (edges, per-file line
//! counts, function counts, duplicate-file groups) into the five root-cause
//! scores. It does not read the filesystem — `sentrux_gate.rs::measure_project`
//! extracts the raw facts (it already reads every file's content once) and
//! calls into this module. Kept filesystem-free specifically so the golden
//! fixtures required by #385's acceptance criteria can drive every formula
//! with plain in-memory inputs instead of temp-directory trees.
//!
//! ## Upstream compatibility, versioned explicitly
//!
//! `FORMULA_VERSION` pins the exact upstream commit this engine was built
//! against (`6f8ff3c14b0423e4b58f42d1813d4d5f7fdc1d11`,
//! `sentrux-core/src/metrics/root_causes.rs`). The normalization formulas
//! (`(Q + 0.5) / 1.5`, `1 / (1 + cycles)`, `1 / (1 + depth / 8)`, `1 - gini`,
//! `1 - ratio`) match both the pinned source and the public docs page
//! (https://sentrux.dev/docs/quality-signal/) exactly. One place the pinned
//! *source* differs from the *docs page*, and this engine follows the
//! source: `compute_root_cause_scores` floors every normalized factor at
//! `0.01` before the geometric-mean product (`values.iter().map(|v|
//! v.max(0.01)).product()`), so one factor collapsing to exactly `0.0`
//! cannot zero out the whole signal. The docs page's formula
//! (`quality_signal = (a*b*c*d*e)^(1/5) * 10000`) omits this floor. This is
//! not silently reproduced or silently dropped: `normalize_and_aggregate`
//! applies the floor (matching the pinned source, which is what #385 names
//! as the reference), and this comment plus `FORMULA_VERSION` is the
//! explicit version record the issue requires.
//!
//! ## Where this engine's own facts fall short of upstream's
//!
//! Upstream's `equality` factor is Gini of *per-function* cyclomatic
//! complexity, "falls back to per-file line counts if no CC data available"
//! (upstream's own documented fallback, `compute_complexity_gini`). This
//! repository's scanner has never tracked per-function complexity — only a
//! per-*file* `branch_density_per_fn` approximation
//! (`sentrux_gate.rs`'s own `branch_density_per_fn` comment: "A true max
//! needs per-function body ranges, which no scanner in this repository
//! has.") So `equality` here always takes upstream's own documented
//! fallback path (per-file LOC), not a new approximation invented for this
//! delivery.
//!
//! Upstream's `redundancy` factor is `(dead + duplicate) / total_functions`.
//! This engine implements the `duplicate` half only: whole-file
//! byte-for-byte duplication after `strip_comments_and_strings` and
//! whitespace normalization, counted at function granularity (every
//! function in a file that exact-matches another file's normalized content
//! counts toward the numerator). `dead` (reachability-based dead-function
//! detection) is **not implemented** — this repository's own `AGENTS.md`
//! already flags naive text-heuristic dead-code detection as actively
//! misleading here (`cargo check`'s ~100 dead-code warnings are a compiler
//! artifact of `#[path]` re-inclusion, not real debt), and a name-based
//! "never called" heuristic would misclassify every `pub` API function,
//! trait impl, test, and framework entry point as dead. Rather than ship a
//! number likely to be *wrong* (worse than absent), `redundancy`'s raw ratio
//! is computed from duplicates only and the gap is reported honestly via
//! `RootCause::completeness` / `RootCause::note` (see `scan`/`health`'s
//! `root_causes.redundancy` field) instead of silently treated as
//! `dead_count = 0`.

use std::collections::{BTreeMap, BTreeSet};

/// Upstream reference commit this formula set was ported from, plus this
/// engine's own explicit deviation tag. Exposed in `scan`/`health` so a
/// consumer can tell which upstream revision (and which documented
/// deviation) produced a given number.
pub(crate) const FORMULA_VERSION: &str =
    "sentrux-upstream@6f8ff3c14b0423e4b58f42d1813d4d5f7fdc1d11+max-floor-0.01";
/// This engine's own version for the root-cause computation kernel,
/// independent of `sentrux_gate::ENGINE_VERSION` (which covers the whole
/// gate engine, including unrelated metrics like `coupling_score`).
pub(crate) const PROVIDER_VERSION: &str = "code-intel-quality-signal.v1";

/// Minimum normalized-content length (bytes) a file needs before it is
/// considered for duplicate-group membership. Below this, near-empty files
/// (blank `__init__.py`, one-line `mod.rs` re-exports) would otherwise
/// collapse into large false-positive duplicate groups purely from
/// boilerplate brevity, not genuine redundancy.
pub(crate) const MIN_DUPLICATE_CONTENT_LEN: usize = 40;

/// Raw (un-normalized) root-cause inputs, already extracted by the caller.
#[derive(Debug, Clone, Default)]
pub(crate) struct RootCauseRaw {
    /// Newman's Q over the resolved file-dependency graph, range [-0.5, 1.0].
    pub(crate) modularity_q: f64,
    /// Count of strongly-connected components with >1 member.
    pub(crate) cycle_count: i64,
    /// Longest path (edge count) through the cycle-condensed dependency DAG.
    pub(crate) max_depth: i64,
    /// Gini coefficient of the equality distribution (per-file LOC, this
    /// engine's honest fallback -- see module doc).
    pub(crate) equality_gini: f64,
    /// (duplicate-only) redundant-function fraction, [0, 1].
    pub(crate) redundancy_ratio: f64,
}

/// One root cause's normalized score plus its raw input, ready to embed in
/// `scan`/`health` JSON.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RootCause {
    /// Normalized score in [0, 1]. Not read by `sentrux_gate.rs` today (the
    /// caller re-derives `raw` from the separately-stored `RootCauseRaw`
    /// instead), kept because it is this kernel's natural pure-math output
    /// and every unit test in this module asserts against it directly.
    #[allow(dead_code)]
    pub(crate) score_unit: f64,
    /// `score_unit` scaled to the 0..10000 display range.
    pub(crate) score: i64,
}

impl RootCause {
    fn new(score_unit: f64) -> Self {
        let clamped = score_unit.clamp(0.0, 1.0);
        RootCause {
            score_unit: clamped,
            score: (clamped * 10000.0).round() as i64,
        }
    }
}

/// Full aggregate result: five normalized root causes, the geometric-mean
/// signal, and the deterministic bottleneck pick.
#[derive(Debug, Clone)]
pub(crate) struct QualitySignal {
    /// Geometric mean of the five root causes, scaled to 0..10000.
    pub(crate) quality_signal: i64,
    /// Name of the lowest-scoring root cause. Ties broken by fixed priority
    /// order (`modularity`, `acyclicity`, `depth`, `equality`,
    /// `redundancy`) -- `Iterator::min_by` returns the first minimal element
    /// on a tie, so this order is also the tie-break order.
    pub(crate) bottleneck: &'static str,
    pub(crate) modularity: RootCause,
    pub(crate) acyclicity: RootCause,
    pub(crate) depth: RootCause,
    pub(crate) equality: RootCause,
    pub(crate) redundancy: RootCause,
}

/// Normalize raw root-cause values and compute the geometric-mean Quality
/// Signal, matching the pinned upstream source
/// (`compute_root_cause_scores`, `root_causes.rs:319-340`) exactly,
/// including its `max(0.01)` per-factor floor (see module doc).
pub(crate) fn normalize_and_aggregate(raw: &RootCauseRaw) -> QualitySignal {
    let modularity = ((raw.modularity_q + 0.5) / 1.5).clamp(0.0, 1.0);
    let acyclicity = 1.0 / (1.0 + raw.cycle_count.max(0) as f64);
    let depth = 1.0 / (1.0 + raw.max_depth.max(0) as f64 / 8.0);
    let equality = (1.0 - raw.equality_gini).clamp(0.0, 1.0);
    let redundancy = (1.0 - raw.redundancy_ratio).clamp(0.0, 1.0);

    // Fixed priority order: also the deterministic bottleneck tie-break
    // order (`Iterator::min_by` keeps the first minimal element on ties).
    let named = [
        ("modularity", modularity),
        ("acyclicity", acyclicity),
        ("depth", depth),
        ("equality", equality),
        ("redundancy", redundancy),
    ];
    let product: f64 = named.iter().map(|(_, value)| value.max(0.01)).product();
    let quality_signal_unit = product.powf(1.0 / 5.0);
    let quality_signal = (quality_signal_unit.clamp(0.0, 1.0) * 10000.0).round() as i64;

    let bottleneck = named
        .iter()
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(name, _)| *name)
        .unwrap_or("modularity");

    QualitySignal {
        quality_signal,
        bottleneck,
        modularity: RootCause::new(modularity),
        acyclicity: RootCause::new(acyclicity),
        depth: RootCause::new(depth),
        equality: RootCause::new(equality),
        redundancy: RootCause::new(redundancy),
    }
}

/// Newman's Modularity Q (Newman 2004) over a directed edge multiset,
/// deduplicated by the caller, with community assignment `module_of`.
///
/// `Q = (1/m) * sum [A_ij - k_out_i * k_in_j / m] * delta(c_i, c_j)`
///
/// Ported from the pinned upstream source (`compute_modularity_q`,
/// `root_causes.rs:72-145`), adapted to take a plain `(from, to)` edge set
/// instead of this repository's `ImportEdge`/`CallEdge`/`FileNode` types
/// (which have no equivalent in this scanner). Isolated nodes (files with
/// no edges) never need to be passed in: a node with in/out degree 0
/// contributes 0 to every module's expected-edge sum, so it cannot change
/// the result -- this mirrors upstream's own `all_nodes` bookkeeping without
/// needing the caller to supply the full file list.
pub(crate) fn compute_modularity_q(edges: &BTreeSet<(String, String)>) -> f64 {
    let m = edges.len();
    if m == 0 {
        return 1.0; // No edges -> trivially modular (nothing connects).
    }
    let m_f = m as f64;

    let mut k_out: BTreeMap<&str, usize> = BTreeMap::new();
    let mut k_in: BTreeMap<&str, usize> = BTreeMap::new();
    for (from, to) in edges {
        *k_out.entry(from.as_str()).or_default() += 1;
        *k_in.entry(to.as_str()).or_default() += 1;
    }

    let mut intra_module_edges: usize = 0;
    for (from, to) in edges {
        if module_of(from) == module_of(to) {
            intra_module_edges += 1;
        }
    }

    let mut mod_k_out_sum: BTreeMap<String, f64> = BTreeMap::new();
    let mut mod_k_in_sum: BTreeMap<String, f64> = BTreeMap::new();
    let mut nodes: BTreeSet<&str> = BTreeSet::new();
    for (from, to) in edges {
        nodes.insert(from.as_str());
        nodes.insert(to.as_str());
    }
    for &node in &nodes {
        let module = module_of(node);
        let ko = *k_out.get(node).unwrap_or(&0) as f64;
        let ki = *k_in.get(node).unwrap_or(&0) as f64;
        *mod_k_out_sum.entry(module.clone()).or_default() += ko;
        *mod_k_in_sum.entry(module).or_default() += ki;
    }

    let mut expected_intra: f64 = 0.0;
    for (module, &ko_sum) in &mod_k_out_sum {
        let ki_sum = mod_k_in_sum.get(module).copied().unwrap_or(0.0);
        expected_intra += ko_sum * ki_sum / m_f;
    }

    let q = (intra_module_edges as f64 - expected_intra) / m_f;
    q.clamp(-0.5, 1.0)
}

/// This engine's community assignment for Newman's Q. Base case: the
/// file's top-level path segment, or the empty string for a file directly
/// at the repository root. One deliberate refinement beyond that base case
/// (independently arrived at, not ported from `sentrux_analysis.rs`'s own
/// `module_name()` / DR-0010, though the shape converges because it is the
/// obvious fix for the same structural pattern DR-0010 names): a
/// `crates/<name>/...` or `packages/<name>/...` path gets one more level of
/// granularity, because without it every file in a single-crate repository
/// (this one included -- `crates/code-intel-cli/src/*.rs`) collapses into
/// one module, making Q trivially ~0 for the exact repositories this
/// engine most needs to say something useful about. A file directly under
/// `<crate>/src|app|tests/` with no further subdirectory is its own module
/// (top-level files are already separate concerns); a file inside a
/// subdirectory shares that subdirectory as its module.
pub(crate) fn module_of(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() <= 1 {
        return String::new(); // File directly at the repository root.
    }
    if let [first, name, root, next, ..] = segments.as_slice() {
        if matches!(*first, "crates" | "packages") && matches!(*root, "src" | "app" | "tests") {
            return format!("{first}/{name}/{root}/{next}");
        }
    }
    if let [first, name, ..] = segments.as_slice() {
        if matches!(*first, "crates" | "packages") {
            return format!("{first}/{name}");
        }
    }
    segments[0].to_string()
}

/// Cycle count and max dependency depth over a resolved file-dependency
/// graph, computed together because both need the same cycle-condensation
/// step. `cycles` is this repository's existing Tarjan SCC output
/// (`sentrux_gate::strongly_connected_cycles`, reused rather than
/// reimplemented) restricted to components with >1 member -- exactly
/// upstream's own `cycle_count` definition.
///
/// `max_depth` is the longest path (edge count) through the graph after
/// collapsing every cycle to a single node. Upstream computes depth via
/// "iterative longest-path DFS from entry points"; that is only
/// well-defined on a DAG. This engine's graphs are not guaranteed acyclic
/// (a real repository can and does have import cycles), so cycles are
/// condensed first -- a deliberate, documented choice beyond upstream's own
/// spec (which assumes acyclic input), needed for this function to
/// terminate and stay bounded on real, possibly-cyclic input.
pub(crate) fn compute_cycles_and_depth(
    edges: &BTreeSet<(String, String)>,
    cycles: &[Vec<String>],
) -> (i64, i64) {
    let cycle_count = cycles.len() as i64;
    if edges.is_empty() {
        return (cycle_count, 0);
    }

    // Map every node inside a cycle group to that group's representative
    // (its lexicographically-first member); every other node maps to
    // itself. This condenses each SCC of size > 1 to one node without
    // needing a full from-scratch SCC computation.
    let mut representative: BTreeMap<&str, &str> = BTreeMap::new();
    for group in cycles {
        let leader = group
            .iter()
            .min()
            .map(String::as_str)
            .expect("cycle groups are non-empty");
        for member in group {
            representative.insert(member.as_str(), leader);
        }
    }
    let rep_of = |node: &str| -> String {
        representative
            .get(node)
            .copied()
            .unwrap_or(node)
            .to_string()
    };

    let mut condensed: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (from, to) in edges {
        let (rf, rt) = (rep_of(from), rep_of(to));
        if rf != rt {
            condensed.entry(rf).or_default().insert(rt);
        }
    }

    // Longest path (edge count) in the now-acyclic condensation graph, via
    // memoized DFS. Guaranteed to terminate: `condensed` has no self-loops
    // (rf != rt filtered above) and no cycles (every genuine cycle was
    // already collapsed into one representative node).
    let mut memo: BTreeMap<String, i64> = BTreeMap::new();
    fn longest_from(
        node: &str,
        condensed: &BTreeMap<String, BTreeSet<String>>,
        memo: &mut BTreeMap<String, i64>,
        visiting: &mut BTreeSet<String>,
    ) -> i64 {
        if let Some(&cached) = memo.get(node) {
            return cached;
        }
        // Defensive guard: should be unreachable given cycles are already
        // condensed, but treating a would-be revisit as depth 0 rather than
        // recursing keeps this function total instead of trusting that
        // invariant unconditionally.
        if !visiting.insert(node.to_string()) {
            return 0;
        }
        let best = condensed
            .get(node)
            .into_iter()
            .flatten()
            .map(|next| 1 + longest_from(next, condensed, memo, visiting))
            .max()
            .unwrap_or(0);
        visiting.remove(node);
        memo.insert(node.to_string(), best);
        best
    }

    let mut nodes: BTreeSet<&str> = BTreeSet::new();
    for (from, to) in edges {
        nodes.insert(from.as_str());
        nodes.insert(to.as_str());
    }
    let mut visiting: BTreeSet<String> = BTreeSet::new();
    let max_depth = nodes
        .iter()
        .map(|&node| {
            let rep = rep_of(node);
            longest_from(&rep, &condensed, &mut memo, &mut visiting)
        })
        .max()
        .unwrap_or(0);

    (cycle_count, max_depth)
}

/// Gini coefficient of a value distribution, ported verbatim from the
/// pinned upstream source (`gini_coefficient`, `root_causes.rs:262-281`).
/// `G = 0`: perfectly equal. `G = 1`: one element has everything. Returns
/// `0.0` for 0/1-element or all-zero inputs (upstream's own guard against a
/// division by an empty/zero total -- never produces NaN).
pub(crate) fn gini(values: &[f64]) -> f64 {
    let n = values.len();
    if n <= 1 {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let total: f64 = sorted.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }

    let mut numerator: f64 = 0.0;
    for (i, &v) in sorted.iter().enumerate() {
        numerator += (2.0 * (i + 1) as f64 - n as f64 - 1.0) * v;
    }
    (numerator / (n as f64 * total)).clamp(0.0, 1.0)
}

/// Redundancy ratio: `duplicate_functions / total_functions`, `0.0` if no
/// functions exist (matches upstream's own `compute_redundancy_ratio`
/// guard). This engine's ratio counts duplicates only -- see module doc for
/// why `dead` is intentionally absent rather than fabricated as `0`.
pub(crate) fn redundancy_ratio(duplicate_functions: i64, total_functions: i64) -> f64 {
    if total_functions <= 0 {
        return 0.0;
    }
    let waste = duplicate_functions.min(total_functions).max(0);
    waste as f64 / total_functions as f64
}

/// Best-effort import/dependency target candidates on one already-detected
/// import-like line (the caller supplies lines matching this repository's
/// existing `is_import_line`). Bounded heuristic, not a parser, consistent
/// with this file's other text-based extractors: prefers a quoted string
/// literal (covers JS/TS relative imports, Go/C/C++ quoted paths); falls
/// back to the last dotted/scoped identifier segment on the line (covers
/// bare-word imports like `use crate::a::b;`, `import a.b.C;`, `using
/// A.B;`). Returns at most one candidate per line -- multi-import lines
/// (`from a import b, c`) are conservatively reduced to their first token
/// rather than guessing a full split.
pub(crate) fn import_target_candidate(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.split_once('"') {
        if let Some((quoted, _)) = rest.1.split_once('"') {
            if !quoted.is_empty() {
                return Some(quoted.to_string());
            }
        }
    }
    if let Some(rest) = trimmed.split_once('\'') {
        if let Some((quoted, _)) = rest.1.split_once('\'') {
            if !quoted.is_empty() {
                return Some(quoted.to_string());
            }
        }
    }

    // No quotes: fall back to the last identifier-ish segment on the line,
    // stripping common trailing syntax first.
    let mut candidate = trimmed;
    for cut in [';', '{', '(', '#'] {
        if let Some((head, _)) = candidate.split_once(cut) {
            candidate = head;
        }
    }
    let candidate = candidate.trim();
    let last_word = candidate.split_whitespace().last()?;
    let last_segment = last_word
        .split(&['.', ':', '/'][..])
        .filter(|segment| !segment.is_empty())
        .next_back()?;
    if last_segment.chars().all(|c| c.is_alphanumeric() || c == '_') && !last_segment.is_empty() {
        Some(last_segment.to_string())
    } else {
        None
    }
}

/// Resolves one raw import target (from `import_target_candidate`) against
/// the known repository file set. A quoted-relative-looking target (starts
/// with `.` or contains `/`) is resolved relative to `from`'s directory,
/// trying a fixed extension list when the bare joined path is not itself a
/// known file. A bare identifier is resolved by matching it against the
/// file-stem (final path segment, extension stripped) of every known path;
/// resolves only when exactly one candidate matches, so an ambiguous bare
/// name (two files named `utils.py` in different directories) is dropped
/// rather than guessed. Unresolved targets (external packages, standard
/// library, ambiguous bare names) return `None`.
pub(crate) fn resolve_edge(
    from: &str,
    raw_target: &str,
    known_paths: &BTreeSet<String>,
) -> Option<String> {
    const CANDIDATE_EXTENSIONS: &[&str] = &[
        "rs", "py", "go", "ts", "tsx", "js", "jsx", "mjs", "cjs", "java", "cs", "cpp", "c", "h",
        "hpp", "v",
    ];
    let target = raw_target.trim();
    if target.is_empty() {
        return None;
    }

    if target.starts_with('.') || target.contains('/') {
        let base_dir = from.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
        let joined = normalize_path_join(base_dir, target);
        if known_paths.contains(&joined) {
            return Some(joined);
        }
        for extension in CANDIDATE_EXTENSIONS {
            let with_ext = format!("{joined}.{extension}");
            if known_paths.contains(&with_ext) {
                return Some(with_ext);
            }
            let as_mod = format!("{joined}/mod.{extension}");
            if known_paths.contains(&as_mod) {
                return Some(as_mod);
            }
            let as_index = format!("{joined}/index.{extension}");
            if known_paths.contains(&as_index) {
                return Some(as_index);
            }
        }
        return None;
    }

    let mut matches = known_paths.iter().filter(|path| file_stem(path) == target);
    let first = matches.next()?;
    if matches.next().is_none() {
        Some(first.clone())
    } else {
        None // Ambiguous: more than one file shares this stem.
    }
}

fn file_stem(path: &str) -> &str {
    let leaf = path.rsplit('/').next().unwrap_or(path);
    leaf.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(leaf)
}

/// Joins a relative import target onto the importing file's directory and
/// resolves `.`/`..` segments, without ever escaping above the repository
/// root (a leading `..` past the root is dropped rather than producing a
/// path outside the scanned tree).
fn normalize_path_join(base_dir: &str, target: &str) -> String {
    let mut segments: Vec<&str> = if base_dir.is_empty() {
        Vec::new()
    } else {
        base_dir.split('/').collect()
    };
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    segments.join("/")
}

/// Normalizes already-comment/string-stripped file content for duplicate
/// detection: collapses all whitespace runs to single spaces and trims.
/// Two files with identical normalized content are treated as duplicates
/// regardless of original formatting/indentation.
pub(crate) fn normalize_for_duplicate_check(stripped_content: &str) -> String {
    let mut normalized = String::with_capacity(stripped_content.len());
    let mut last_was_space = true; // Suppresses a leading space.
    for ch in stripped_content.chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                normalized.push(' ');
                last_was_space = true;
            }
        } else {
            normalized.push(ch);
            last_was_space = false;
        }
    }
    if normalized.ends_with(' ') {
        normalized.pop();
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edges(pairs: &[(&str, &str)]) -> BTreeSet<(String, String)> {
        pairs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    // ── Single-factor fixtures: each root cause becomes the sole
    // bottleneck when the other four are held at their perfect value. ──

    #[test]
    fn modularity_is_the_sole_bottleneck_when_it_is_the_only_weak_factor() {
        let raw = RootCauseRaw {
            modularity_q: -0.5, // worst possible Q -> modularity score 0.0
            cycle_count: 0,
            max_depth: 0,
            equality_gini: 0.0,
            redundancy_ratio: 0.0,
        };
        let result = normalize_and_aggregate(&raw);
        assert_eq!(result.bottleneck, "modularity");
        assert_eq!(result.modularity.score, 0);
        assert_eq!(result.acyclicity.score, 10000);
        assert_eq!(result.depth.score, 10000);
        assert_eq!(result.equality.score, 10000);
        assert_eq!(result.redundancy.score, 10000);
    }

    #[test]
    fn acyclicity_is_the_sole_bottleneck_when_it_is_the_only_weak_factor() {
        let raw = RootCauseRaw {
            modularity_q: 1.0,
            cycle_count: 99, // many cycles -> acyclicity score near 0
            max_depth: 0,
            equality_gini: 0.0,
            redundancy_ratio: 0.0,
        };
        let result = normalize_and_aggregate(&raw);
        assert_eq!(result.bottleneck, "acyclicity");
        assert!(result.acyclicity.score < result.modularity.score);
    }

    #[test]
    fn depth_is_the_sole_bottleneck_when_it_is_the_only_weak_factor() {
        let raw = RootCauseRaw {
            modularity_q: 1.0,
            cycle_count: 0,
            max_depth: 999, // very deep chain -> depth score near 0
            equality_gini: 0.0,
            redundancy_ratio: 0.0,
        };
        let result = normalize_and_aggregate(&raw);
        assert_eq!(result.bottleneck, "depth");
    }

    #[test]
    fn equality_is_the_sole_bottleneck_when_it_is_the_only_weak_factor() {
        let raw = RootCauseRaw {
            modularity_q: 1.0,
            cycle_count: 0,
            max_depth: 0,
            equality_gini: 1.0, // one element has everything -> equality 0
            redundancy_ratio: 0.0,
        };
        let result = normalize_and_aggregate(&raw);
        assert_eq!(result.bottleneck, "equality");
        assert_eq!(result.equality.score, 0);
    }

    #[test]
    fn redundancy_is_the_sole_bottleneck_when_it_is_the_only_weak_factor() {
        let raw = RootCauseRaw {
            modularity_q: 1.0,
            cycle_count: 0,
            max_depth: 0,
            equality_gini: 0.0,
            redundancy_ratio: 1.0, // everything redundant -> redundancy 0
        };
        let result = normalize_and_aggregate(&raw);
        assert_eq!(result.bottleneck, "redundancy");
        assert_eq!(result.redundancy.score, 0);
    }

    #[test]
    fn bottleneck_tie_break_is_deterministic_priority_order() {
        // All five factors tied at their worst -> `modularity` wins by
        // fixed priority order, not iteration-order luck.
        let raw = RootCauseRaw {
            modularity_q: -0.5,
            cycle_count: 0, // acyclicity would be 1.0 here; force a real tie instead:
            max_depth: 0,
            equality_gini: 1.0,
            redundancy_ratio: 1.0,
        };
        // Force acyclicity/depth down to the same floor as the others by
        // picking cycle_count/max_depth so 1/(1+x) == the modularity score.
        // modularity score = (−0.5+0.5)/1.5 = 0.0 exactly; acyclicity can
        // only equal 0.0 in the limit, never exactly, with a finite count,
        // so instead assert the documented order directly against a clean
        // symmetric tie between equality and redundancy (both exactly 0.0).
        let result = normalize_and_aggregate(&raw);
        assert_eq!(result.equality.score, 0);
        assert_eq!(result.redundancy.score, 0);
        // modularity (priority 0) is even lower (also 0) so it must win.
        assert_eq!(result.bottleneck, "modularity");

        let raw_tied = RootCauseRaw {
            modularity_q: 1.0,
            cycle_count: 0,
            max_depth: 0,
            equality_gini: 1.0,
            redundancy_ratio: 1.0,
        };
        let tied = normalize_and_aggregate(&raw_tied);
        assert_eq!(tied.equality.score, 0);
        assert_eq!(tied.redundancy.score, 0);
        // equality (priority 3) precedes redundancy (priority 4).
        assert_eq!(tied.bottleneck, "equality");
    }

    // ── Zero / empty / unknown-input edge cases: no NaN, no panics. ──

    #[test]
    fn empty_graph_is_trivially_modular_and_acyclic() {
        let empty = edges(&[]);
        assert_eq!(compute_modularity_q(&empty), 1.0);
        let (cycle_count, max_depth) = compute_cycles_and_depth(&empty, &[]);
        assert_eq!(cycle_count, 0);
        assert_eq!(max_depth, 0);
    }

    #[test]
    fn zero_and_single_element_gini_is_zero_not_nan() {
        assert_eq!(gini(&[]), 0.0);
        assert_eq!(gini(&[42.0]), 0.0);
        assert_eq!(gini(&[0.0, 0.0, 0.0]), 0.0); // all-zero total guard
    }

    #[test]
    fn zero_functions_redundancy_ratio_is_zero_not_nan() {
        assert_eq!(redundancy_ratio(0, 0), 0.0);
        assert_eq!(redundancy_ratio(5, 0), 0.0);
    }

    #[test]
    fn full_aggregate_never_produces_nan_or_inf_on_empty_input() {
        let raw = RootCauseRaw::default();
        let result = normalize_and_aggregate(&raw);
        assert!(result.quality_signal >= 0 && result.quality_signal <= 10000);
        for score in [
            result.modularity.score,
            result.acyclicity.score,
            result.depth.score,
            result.equality.score,
            result.redundancy.score,
        ] {
            assert!((0..=10000).contains(&score));
        }
    }

    // ── Known-value fixtures pinning the ported formulas. ──

    #[test]
    fn gini_equal_distribution_is_zero() {
        assert!(gini(&[10.0, 10.0, 10.0, 10.0]).abs() < 0.01);
    }

    #[test]
    fn gini_unequal_distribution_is_high() {
        assert!(gini(&[0.0, 0.0, 0.0, 100.0]) > 0.6);
    }

    #[test]
    fn root_cause_scores_match_pinned_upstream_fixture() {
        // Same inputs as the pinned upstream source's own
        // `root_cause_scores_normalize` test (root_causes.rs:378-394).
        let raw = RootCauseRaw {
            modularity_q: 0.5,
            cycle_count: 0,
            max_depth: 4,
            equality_gini: 0.2,
            redundancy_ratio: 0.1,
        };
        let result = normalize_and_aggregate(&raw);
        assert!(result.modularity.score_unit > 0.6);
        assert_eq!(result.acyclicity.score_unit, 1.0);
        assert!(result.depth.score_unit > 0.5);
        assert!(result.equality.score_unit > 0.7);
        assert!(result.redundancy.score_unit > 0.8);
        assert!(result.quality_signal as f64 / 10000.0 > 0.6);
    }

    #[test]
    fn max_floor_prevents_one_zero_factor_from_zeroing_the_whole_signal() {
        // Docs-page formula (no floor) would give exactly 0. The pinned
        // source's max(0.01) floor keeps the geometric mean positive.
        let raw = RootCauseRaw {
            modularity_q: -0.5, // modularity score exactly 0.0
            cycle_count: 0,
            max_depth: 0,
            equality_gini: 0.0,
            redundancy_ratio: 0.0,
        };
        let result = normalize_and_aggregate(&raw);
        assert!(result.quality_signal > 0, "floor must keep signal positive");
    }

    // ── Graph algorithm fixtures. ──

    #[test]
    fn modularity_q_two_isolated_clusters_is_high() {
        // a1<->a2 within module "a", b1<->b2 within module "b": every edge
        // is intra-module, so Q should be strongly positive.
        let e = edges(&[
            ("a/1.rs", "a/2.rs"),
            ("a/2.rs", "a/1.rs"),
            ("b/1.rs", "b/2.rs"),
            ("b/2.rs", "b/1.rs"),
        ]);
        let q = compute_modularity_q(&e);
        assert!(q > 0.3, "expected strong modular structure, got {q}");
    }

    #[test]
    fn module_of_uses_top_level_path_segment() {
        assert_eq!(module_of("src/a/b.rs"), "src");
        assert_eq!(module_of("lib.rs"), "");
    }

    #[test]
    fn module_of_gives_crates_paths_one_more_level_of_granularity() {
        // Flat top-level file: its own module, not the whole crate.
        assert_eq!(
            module_of("crates/code-intel-cli/src/sentrux_gate.rs"),
            "crates/code-intel-cli/src/sentrux_gate.rs"
        );
        // Two files under the same subdirectory: same module.
        assert_eq!(
            module_of("crates/code-intel-cli/src/graph/mod.rs"),
            "crates/code-intel-cli/src/graph"
        );
        assert_eq!(
            module_of("crates/code-intel-cli/src/graph/tests.rs"),
            "crates/code-intel-cli/src/graph"
        );
        // Outside src/app/tests: falls back to the coarse crate bucket.
        assert_eq!(
            module_of("crates/code-intel-cli/build.rs"),
            "crates/code-intel-cli"
        );
    }

    #[test]
    fn two_file_mutual_cycle_yields_depth_one_after_condensation() {
        let e = edges(&[("a.rs", "b.rs"), ("b.rs", "a.rs")]);
        let cycles = vec![vec!["a.rs".to_string(), "b.rs".to_string()]];
        let (cycle_count, max_depth) = compute_cycles_and_depth(&e, &cycles);
        assert_eq!(cycle_count, 1);
        // Collapsed to a single node with a self-loop removed -> depth 0.
        assert_eq!(max_depth, 0);
    }

    #[test]
    fn linear_chain_depth_matches_edge_count() {
        let e = edges(&[("a.rs", "b.rs"), ("b.rs", "c.rs"), ("c.rs", "d.rs")]);
        let (cycle_count, max_depth) = compute_cycles_and_depth(&e, &[]);
        assert_eq!(cycle_count, 0);
        assert_eq!(max_depth, 3);
    }

    #[test]
    fn cycle_plus_tail_condenses_then_continues() {
        // a<->b cycle, then b->c: condensed graph is one edge, depth 1.
        let e = edges(&[("a.rs", "b.rs"), ("b.rs", "a.rs"), ("b.rs", "c.rs")]);
        let cycles = vec![vec!["a.rs".to_string(), "b.rs".to_string()]];
        let (cycle_count, max_depth) = compute_cycles_and_depth(&e, &cycles);
        assert_eq!(cycle_count, 1);
        assert_eq!(max_depth, 1);
    }

    // ── Import target extraction / resolution fixtures. ──

    #[test]
    fn quoted_relative_import_resolves_against_known_paths() {
        let known: BTreeSet<String> = ["src/a.ts", "src/b.ts"].iter().map(|s| s.to_string()).collect();
        let candidate = import_target_candidate("import { x } from \"./b\";").unwrap();
        assert_eq!(candidate, "./b");
        let resolved = resolve_edge("src/a.ts", &candidate, &known);
        assert_eq!(resolved.as_deref(), Some("src/b.ts"));
    }

    #[test]
    fn bare_identifier_resolves_only_when_unambiguous() {
        let known: BTreeSet<String> = ["src/foo.rs", "other/foo.rs"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // Two files named foo.* -> ambiguous, must not guess.
        assert_eq!(resolve_edge("root.rs", "foo", &known), None);

        let unambiguous: BTreeSet<String> = ["src/bar.rs"].iter().map(|s| s.to_string()).collect();
        assert_eq!(
            resolve_edge("root.rs", "bar", &unambiguous).as_deref(),
            Some("src/bar.rs")
        );
    }

    #[test]
    fn unresolvable_targets_are_dropped_not_guessed() {
        let known: BTreeSet<String> = ["src/a.rs"].iter().map(|s| s.to_string()).collect();
        assert_eq!(resolve_edge("src/b.rs", "std::fs", &known), None);
    }

    #[test]
    fn duplicate_normalization_collapses_whitespace_differences() {
        let a = normalize_for_duplicate_check("pub fn f() {\n    1\n}\n");
        let b = normalize_for_duplicate_check("pub fn f() {\n\t1\n}");
        assert_eq!(a, b);
    }
}
