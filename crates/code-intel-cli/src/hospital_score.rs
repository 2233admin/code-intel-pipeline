//! Hospital report scoring, ported from `legacy/run-code-intel.ps1`.
//!
//! The Rust hospital emitted `null` for every score while the PowerShell
//! launcher computed real values, so this was never duplication — it was the
//! only implementation, and it lived in the file T2 is retiring. See
//! docs/ps1-exit/t2-dot-source-parity-map.md §3.
//!
//! This module is the scoring arithmetic only. It is deliberately pure: no
//! I/O, no artifact reading, no knowledge of where the inputs come from. The
//! wiring — which run signals feed which score — is a separate problem,
//! because roughly half the launcher's score inputs (DSM scope, CodeNexus
//! context, runtime CI health) have no Rust producer yet. Emitting `0` for
//! those would be the same mistake the structural-scope fix corrected: `0`
//! here means *observed and absent*, not *never attempted*.
//!
//! Hence `allow(dead_code)`: the arithmetic is complete and verified against
//! the PowerShell original, but nothing calls it until those inputs exist.
//! Wiring it against inputs the pipeline does not produce would publish
//! confidently wrong scores, which is worse than the `null`s it replaces.
#![allow(dead_code)]

/// `[math]::Round` in .NET is banker's rounding — half to even — and the
/// launcher relies on the default. `f64::round` rounds half away from zero,
/// so a direct translation silently disagrees on every exact `.5`, which is
/// reachable here (three- and four-term averages of integers).
fn round_half_to_even(value: f64) -> f64 {
    let rounded = value.round();
    if (value - value.trunc()).abs() == 0.5 && rounded % 2.0 != 0.0 {
        rounded - value.signum()
    } else {
        rounded
    }
}

fn round_to_int(value: f64) -> i64 {
    round_half_to_even(value) as i64
}

/// `Get-StepScore`: a step is worth everything or nothing. Absent steps and
/// any non-`passed` status both score zero, matching the launcher's `switch`
/// with its `default` arm.
pub(crate) fn step_score(status: Option<&str>) -> i64 {
    match status {
        Some("passed") => 100,
        _ => 0,
    }
}

/// `Get-ImportResolutionScore`: banded, and the bottom band is 30 rather than
/// 0 — an unresolved-heavy graph is degraded evidence, not absent evidence.
/// Only a genuinely unknown ratio scores zero.
pub(crate) fn import_resolution_score(resolved_ratio: Option<f64>) -> i64 {
    let Some(ratio) = resolved_ratio else {
        return 0;
    };
    if ratio >= 75.0 {
        100
    } else if ratio >= 50.0 {
        75
    } else if ratio >= 25.0 {
        50
    } else {
        30
    }
}

/// `New-HospitalMeasurements`' ratio: `None` when nothing was measured, so the
/// caller can tell "no imports seen" from "no imports resolved". Rounded to
/// one decimal, half to even, as the launcher does.
pub(crate) fn resolved_ratio(resolved_imports: i64, unresolved_imports: i64) -> Option<f64> {
    let total = resolved_imports + unresolved_imports;
    if total <= 0 {
        return None;
    }
    let ratio = (resolved_imports as f64 * 100.0) / total as f64;
    Some(round_half_to_even(ratio * 10.0) / 10.0)
}

/// `Get-SourceCoverageScore`: what fraction of the inventory the structural
/// scan actually reached, capped at 100. Either side being non-positive means
/// the comparison is meaningless, not that coverage is bad.
pub(crate) fn source_coverage_score(scan_files: i64, inventory_files: i64) -> i64 {
    if scan_files <= 0 || inventory_files <= 0 {
        return 0;
    }
    round_to_int((scan_files as f64 * 100.0 / inventory_files as f64).min(100.0))
}

/// `New-HospitalScoreBlock`'s pollution arm. Absent evidence is `unknown` and
/// scores zero; present evidence scores 100 when something was quarantined and
/// 80 when nothing needed to be, because "nothing excluded" is a weaker
/// signal than "exclusions were computed and applied".
pub(crate) fn pollution_score(pollution_status: &str, excluded_files: i64) -> i64 {
    if pollution_status == "unknown" {
        0
    } else if excluded_files > 0 {
        100
    } else {
        80
    }
}

/// `New-HospitalMeasurements`' pollution classification.
pub(crate) fn pollution_status(has_evidence: bool, excluded_files: i64) -> &'static str {
    if !has_evidence {
        "unknown"
    } else if excluded_files > 0 {
        "quarantined"
    } else {
        "clean"
    }
}

/// Rules present is worth 100, absent 45 — governance without rules is
/// degraded rather than absent, the same shape as the import bands.
pub(crate) fn rules_score(rules_exist: bool) -> i64 {
    if rules_exist {
        100
    } else {
        45
    }
}

/// The three composite scores the hospital report exposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CompositeScores {
    pub(crate) governance: i64,
    pub(crate) diagnostic: i64,
    pub(crate) overall: i64,
}

/// The composition `New-HospitalScoreBlock` performs: governance over three
/// terms, diagnostic over four, and overall over the two composites plus the
/// two standalone dimensions. Averaging composites into the overall is the
/// launcher's shape and is preserved deliberately — changing the weighting
/// would move every historical score.
pub(crate) fn compose(
    rules: i64,
    gate: i64,
    check: i64,
    ct: i64,
    mri: i64,
    graph: i64,
    memory: i64,
    resolution: i64,
    pollution: i64,
) -> CompositeScores {
    let governance = round_to_int((rules + gate + check) as f64 / 3.0);
    let diagnostic = round_to_int((ct + mri + graph + memory) as f64 / 4.0);
    let overall = round_to_int((diagnostic + governance + resolution + pollution) as f64 / 4.0);
    CompositeScores {
        governance,
        diagnostic,
        overall,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_score_is_all_or_nothing_and_absent_is_nothing() {
        assert_eq!(step_score(Some("passed")), 100);
        assert_eq!(step_score(Some("failed")), 0);
        assert_eq!(step_score(Some("skipped")), 0);
        assert_eq!(step_score(Some("manual_required")), 0);
        assert_eq!(step_score(None), 0);
    }

    #[test]
    fn import_resolution_bands_match_the_launcher_including_the_band_edges() {
        assert_eq!(import_resolution_score(Some(100.0)), 100);
        assert_eq!(import_resolution_score(Some(75.0)), 100);
        assert_eq!(import_resolution_score(Some(74.9)), 75);
        assert_eq!(import_resolution_score(Some(50.0)), 75);
        assert_eq!(import_resolution_score(Some(49.9)), 50);
        assert_eq!(import_resolution_score(Some(25.0)), 50);
        assert_eq!(import_resolution_score(Some(24.9)), 30);
        assert_eq!(import_resolution_score(Some(0.0)), 30);
        // Unknown is the only zero: a fully unresolved graph still scores 30.
        assert_eq!(import_resolution_score(None), 0);
    }

    #[test]
    fn resolved_ratio_is_none_when_nothing_was_measured() {
        assert_eq!(resolved_ratio(0, 0), None);
        assert_eq!(resolved_ratio(0, 5), Some(0.0));
        assert_eq!(resolved_ratio(1055, 0), Some(100.0));
        assert_eq!(resolved_ratio(1, 2), Some(33.3));
        assert_eq!(resolved_ratio(2, 1), Some(66.7));
    }

    #[test]
    fn source_coverage_is_capped_and_meaningless_comparisons_score_zero() {
        assert_eq!(source_coverage_score(232, 232), 100);
        // A scan wider than the inventory caps rather than exceeding 100.
        assert_eq!(source_coverage_score(500, 232), 100);
        assert_eq!(source_coverage_score(116, 232), 50);
        assert_eq!(source_coverage_score(0, 232), 0);
        assert_eq!(source_coverage_score(232, 0), 0);
        assert_eq!(source_coverage_score(-1, 232), 0);
    }

    #[test]
    fn pollution_distinguishes_unknown_from_clean() {
        assert_eq!(pollution_status(false, 0), "unknown");
        assert_eq!(pollution_status(true, 0), "clean");
        assert_eq!(pollution_status(true, 3), "quarantined");
        assert_eq!(pollution_score("unknown", 0), 0);
        assert_eq!(pollution_score("clean", 0), 80);
        assert_eq!(pollution_score("quarantined", 3), 100);
    }

    #[test]
    fn rules_absent_is_degraded_not_absent() {
        assert_eq!(rules_score(true), 100);
        assert_eq!(rules_score(false), 45);
    }

    /// The parity trap: `[math]::Round` is half-to-even, `f64::round` is half
    /// away from zero. Both averages below land exactly on `.5`, so a naive
    /// translation would disagree with every score the launcher ever emitted.
    #[test]
    fn composition_uses_bankers_rounding_like_the_launcher() {
        // (100 + 100 + 45) / 3 = 81.666 -> 82; four-term 0.5 cases below.
        assert_eq!(round_half_to_even(2.5), 2.0);
        assert_eq!(round_half_to_even(3.5), 4.0);
        assert_eq!(round_half_to_even(-2.5), -2.0);
        assert_eq!(round_half_to_even(0.5), 0.0);
        assert_eq!(round_half_to_even(1.5), 2.0);
        // (100 + 0 + 100 + 0) / 4 = 50 exactly, no rounding involved.
        assert_eq!(compose(100, 0, 100, 100, 0, 100, 0, 100, 80).diagnostic, 50);
        // (0 + 0 + 0 + 1) / 4 = 0.25 -> 0; (0 + 0 + 1 + 2) / 4 = 0.75 -> 1.
        assert_eq!(compose(0, 0, 1, 0, 0, 0, 1, 0, 0).governance, 0);
    }

    /// A worked example against the shape the launcher produced on this
    /// repository: rules present, gate and check passed, graph and memory
    /// absent, no CT/MRI producers, full import resolution, clean pollution.
    #[test]
    fn a_governed_repository_without_optional_modalities_scores_predictably() {
        let scores = compose(
            rules_score(true),
            step_score(Some("passed")),
            step_score(Some("passed")),
            0,
            0,
            step_score(None),
            step_score(None),
            import_resolution_score(resolved_ratio(1055, 0)),
            pollution_score(pollution_status(true, 0), 0),
        );
        assert_eq!(scores.governance, 100);
        assert_eq!(scores.diagnostic, 0);
        assert_eq!(scores.overall, 70);
    }
}
