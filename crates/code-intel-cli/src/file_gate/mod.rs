//! Shared candidate-file gate engine for `sentrux scan` and `sentrux dsm`
//! (issue #152).
//!
//! Issue #148 C2 measured a real self-scan where `sentrux scan` reported
//! `"files": 277` and `sentrux dsm` reported `"included_files": 315` on the
//! *same tree*, with only 2 exclusions ever named. 38 files vanished with no
//! attribution, because each command grew its own file-collection walk with
//! its own exclusion rules, and the two quietly drifted:
//!
//!   - `sentrux dsm`'s old `excluded_reason` matched `tools` / `vendor` /
//!     `third_party` / `external` only against the *first* path segment
//!     (`parts.first()`), so `legacy/tools/*.ps1` was invisible to that
//!     check and silently counted as included. `sentrux scan`'s
//!     `SKIP_DIRECTORIES` had always matched at any depth. On this
//!     repository's own tree that one gap alone accounted for 39 files
//!     (measured empirically while implementing this fix).
//!   - `sentrux dsm`'s `SOURCE_EXTENSIONS` (14 extensions) was missing
//!     `cpp` / `c` / `h` / `hpp`, which `sentrux scan`'s `CODE_EXTENSIONS`
//!     (18 extensions) already recognised, so a matching file was dropped
//!     with no count anywhere -- not included, not excluded, just absent.
//!
//! Both gaps are silent-drop bugs: a file that should have been walked
//! simply never became a decision at all in one of the two commands. The
//! fix in this module is not "reconcile the two numbers" but "make the two
//! commands call the same function": `sentrux_gate::measure_project` and
//! `sentrux_analysis::source_inventory` both call [`evaluate`] on the same
//! tree with the same [`GateConfig`], so their file counts are one
//! computation, not two separately maintained ones that happen to agree
//! today. A future edit to the exclusion rules only has one place to make
//! it, so the 277/315 split is now structurally impossible, not merely
//! fixed once.
//!
//! # The gate chain
//!
//! Every candidate file gets exactly one [`GateDecision`]: `{path, decision,
//! gate, source}`. Gate names are part of the JSON contract
//! ([`GateReport::to_json`]), not a log string. [`GATE_ORDER`] is the
//! declared precedence -- first match wins:
//!
//! 1. `unsupported_ext` -- extension not in [`CODE_EXTENSIONS`]. Checked
//!    first because it is a free, in-memory string comparison: there is no
//!    reason to pay for path-matching or file I/O on a file that is
//!    excluded regardless of anything else.
//! 2. `user_include` -- explicit user override. If the path matches a
//!    configured include pattern, the file is *tentatively* included,
//!    which grants immunity from the two path-based exclude gates below
//!    (`user_exclude`, `default_path`) and from `repository_ignored` --
//!    that is what "explicit include un-excludes" means. It does **not**
//!    grant immunity from `binary` (see below): forcing a genuinely binary
//!    file to "included" would make downstream content reads
//!    (`fs::read_to_string` in `sentrux_analysis`) fail the whole run, and
//!    no reasonable include pattern is meant to promise "treat this
//!    binary blob as source text".
//! 3. `user_exclude` -- explicit deny pattern (skipped if step 2 already
//!    granted tentative inclusion).
//! 4. `default_path` -- built-in default directory/path exclusions (skipped
//!    if step 2 already granted tentative inclusion): [`DEFAULT_EXCLUDE_DIRS`]
//!    matched at *any* path depth (the depth-scoping bug fix above), plus
//!    the generic "any hidden directory" rule and the bundled/static-asset
//!    leaf-name rule `sentrux scan` already had.
//! 5. `oversized` -- file bigger than [`MAX_FILE_BYTES`] (skipped if step 2
//!    already granted tentative inclusion). Named separately from
//!    `default_path` because the reason a huge file is excluded is its
//!    size, not its location; folding it into `default_path` would make a
//!    decision record lie about why a specific file was dropped.
//! 6. `repository_ignored` -- gitignored, or untracked and not visible to
//!    ripgrep (skipped if step 2 already granted tentative inclusion).
//!    Sourced from the project's own checked-in `.gitignore`, so its
//!    `source` is `"project"`, not `"built_in"` -- see [`SOURCE_PROJECT`].
//! 7. `binary` -- the file's content looks binary (a NUL byte in the first
//!    `BINARY_SNIFF_BYTES` bytes). Checked *last*, deliberately: every
//!    gate above it is a cheap in-memory or path check that can rule a
//!    candidate out for free, so content is read only for files that would
//!    otherwise be included -- and this is the one gate `user_include`
//!    cannot override, protecting the UTF-8 content reads downstream.
//! 8. `default_include` -- nothing above matched; an ordinary recognised
//!    source file.
//!
//! # The identity
//!
//! `candidates == included + sum(excluded by gate)` by construction: every
//! candidate produces exactly one [`GateDecision`], partitioned into
//! exactly the two buckets. [`GateReport::verify_identity`] checks this
//! structurally rather than trusting the arithmetic, so an actual coding
//! defect (a candidate silently skipped, one double-recorded) fails the
//! command outright -- fail-closed, not a warning execution continues past
//! (issue #152 requirement 3). [`evaluate`] calls it before returning, so
//! no caller can forget to check.
//!
//! # Where each gate's rule comes from
//!
//! [`SOURCE_BUILT_IN`] / [`SOURCE_PROJECT`] declare which layer supplied the
//! rule that decided a candidate (issue #152 requirement 5). Today there are
//! two live layers: the hardcoded defaults in [`rules`] (`built_in`), and
//! the repository's own `.gitignore` (`project`, via `repository_ignored`).
//! [`GateConfig::user_exclude`] / [`GateConfig::user_include`] are real,
//! ordered, tested gates -- see `tests` -- but no project config file or CLI
//! flag reads into them yet, so [`GateConfig::built_in`] always constructs
//! them empty. Every real run's `user_exclude` / `user_include` gate count
//! is honestly 0 until a config surface is wired; the gate chain and JSON
//! contract already have the slot reserved rather than needing another
//! contract change when one lands. There is deliberately no `invocation`
//! source constant yet: no CLI flag exists to populate one (see the
//! code-intel-pipeline#152 implementation report).
//!
//! ## Layout
//!
//! Originally one file; split (still issue #152, before landing) once it
//! crossed this repository's own `loc > 800` monolith rule -- the honesty
//! fix cannot itself ship as the thing it polices. [`rules`] holds the gate
//! names, shared extension/directory data, and the pure per-candidate
//! predicates; [`walk`] holds the filesystem traversal and the `rg`/`git`
//! integration behind `repository_ignored`; [`report`] holds [`GateReport`],
//! the identity check, and the JSON projection. This file keeps the gate
//! order rationale above and the single [`evaluate`] entry point that ties
//! the three together -- the same split shape `change_risk` and
//! `change_agenda` already use (`git`/`signals`/`scoring`/`render`,
//! `cochange`/`cluster`/`render`).

use std::path::Path;

mod report;
mod rules;
#[cfg(test)]
mod tests;
mod walk;

pub(crate) use report::GateReport;
use rules::{default_path_match, extension_of, is_oversized, looks_binary, matches_pattern};
pub(crate) use rules::{
    Decision, GateConfig, GateDecision, CODE_EXTENSIONS, GATE_BINARY, GATE_DEFAULT_INCLUDE,
    GATE_DEFAULT_PATH, GATE_OVERSIZED, GATE_REPOSITORY_IGNORED, GATE_UNSUPPORTED_EXT,
    GATE_USER_EXCLUDE, GATE_USER_INCLUDE, SOURCE_BUILT_IN, SOURCE_PROJECT,
};
use walk::{governed_visible_files, walk_candidates};

/// Evaluate every candidate file under `repo` against the shared gate chain
/// and return the full per-candidate decision record. Both `sentrux scan`
/// (`sentrux_gate::measure_project`) and `sentrux dsm`
/// (`sentrux_analysis::source_inventory`) call this same function on the
/// same tree with the same config, so their file counts are the same
/// computation, not two separately maintained ones that happen to agree
/// today (issue #148 C2).
pub(crate) fn evaluate(repo: &Path, config: &GateConfig) -> Result<GateReport, String> {
    let mut candidates = Vec::new();
    walk_candidates(repo, repo, &mut candidates)?;
    candidates.sort();
    candidates.dedup();

    let governed_visible = governed_visible_files(repo);

    let mut decisions = Vec::with_capacity(candidates.len());
    let mut included = Vec::new();
    for relative in &candidates {
        let extension = extension_of(relative);
        let decision = if !CODE_EXTENSIONS.contains(&extension.as_str()) {
            GateDecision {
                path: relative.clone(),
                decision: Decision::Excluded,
                gate: GATE_UNSUPPORTED_EXT,
                source: SOURCE_BUILT_IN,
            }
        } else {
            let user_included = matches_pattern(relative, &config.user_include);
            if !user_included && matches_pattern(relative, &config.user_exclude) {
                GateDecision {
                    path: relative.clone(),
                    decision: Decision::Excluded,
                    gate: GATE_USER_EXCLUDE,
                    source: SOURCE_BUILT_IN,
                }
            } else if !user_included && default_path_match(relative) {
                GateDecision {
                    path: relative.clone(),
                    decision: Decision::Excluded,
                    gate: GATE_DEFAULT_PATH,
                    source: SOURCE_BUILT_IN,
                }
            } else if !user_included && is_oversized(repo, relative) {
                GateDecision {
                    path: relative.clone(),
                    decision: Decision::Excluded,
                    gate: GATE_OVERSIZED,
                    source: SOURCE_BUILT_IN,
                }
            } else if !user_included
                && governed_visible
                    .as_ref()
                    .is_some_and(|visible| !visible.contains(relative))
            {
                GateDecision {
                    path: relative.clone(),
                    decision: Decision::Excluded,
                    gate: GATE_REPOSITORY_IGNORED,
                    source: SOURCE_PROJECT,
                }
            } else if looks_binary(&repo.join(relative)) {
                GateDecision {
                    path: relative.clone(),
                    decision: Decision::Excluded,
                    gate: GATE_BINARY,
                    source: SOURCE_BUILT_IN,
                }
            } else if user_included {
                GateDecision {
                    path: relative.clone(),
                    decision: Decision::Included,
                    gate: GATE_USER_INCLUDE,
                    source: SOURCE_BUILT_IN,
                }
            } else {
                GateDecision {
                    path: relative.clone(),
                    decision: Decision::Included,
                    gate: GATE_DEFAULT_INCLUDE,
                    source: SOURCE_BUILT_IN,
                }
            }
        };
        if decision.decision == Decision::Included {
            included.push(relative.clone());
        }
        decisions.push(decision);
    }
    included.sort();

    let report = GateReport {
        candidates: candidates.len() as i64,
        included,
        decisions,
    };
    report.verify_identity()?;
    Ok(report)
}
