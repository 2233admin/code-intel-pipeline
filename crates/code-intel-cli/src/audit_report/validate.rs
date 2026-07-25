use std::{fs, path::Path};

use super::{
    enums::{Coverage, DepartmentRunStatus, EvidenceKind, FindingStatus},
    model::AuditReport,
    registry::{repo_relative_path, DepartmentRegistry},
};

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn nullable_f64_eq(left: Option<f64>, right: Option<f64>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => (left - right).abs() < 1e-9,
        _ => false,
    }
}

fn is_perfect_score(score: f64) -> bool {
    (score - 10.0).abs() < 1e-9
}

impl AuditReport {
    /// Fail-closed invariants beyond what the JSON Schema can express.
    /// Each branch below is one distinct, independently testable rule:
    ///
    /// (a) a `status: confirmed` finding has at least one `file`-kind
    ///     evidence entry with a `path`;
    /// (b) every finding/score/coverage department reference is a
    ///     registered department id, and every score/coverage department
    ///     reference is additionally present in this report's own
    ///     `departments` list (not merely registered — see (c));
    /// (c) `self.departments` exactly matches the registry: no registered
    ///     department is missing a run, and no run names an unregistered
    ///     department;
    /// (d) a department run's `status` is consistent with the registry's
    ///     `enabled` flag: `enabled: false` requires `status: disabled`,
    ///     and `enabled: true` requires `status: assessed` or
    ///     `not_assessed` (never `disabled`);
    /// (e) a `not_assessed`/`disabled` department has a null score in
    ///     `score_dashboard` and a `not_assessed` row in `coverage_matrix`;
    /// (f) `score_dashboard.overall` equals the recomputed mean of the
    ///     non-null entry scores, rounded to 1 decimal, or `null` when
    ///     nothing scored;
    /// (g) a department that scores a perfect `10.0` with zero findings has
    ///     `coverage: high`;
    /// (h) every report department has exactly one `coverage_matrix` row and
    ///     at most one `score_dashboard` entry;
    /// (i) an `assessed` department actually moves the health score: it has
    ///     a non-null `score_dashboard` entry and its coverage row is not
    ///     `not_assessed`.
    pub(crate) fn validate(&self, registry: &DepartmentRegistry) -> Result<(), String> {
        // (a) every finding has evidence (structural, enforced at parse time
        // via minItems); a confirmed finding also needs a file+path entry.
        for finding in &self.findings {
            if finding.status == FindingStatus::Confirmed
                && !finding
                    .evidence
                    .iter()
                    .any(|entry| entry.kind == EvidenceKind::File && entry.path.is_some())
            {
                return Err(format!(
                    "finding \"{}\" is confirmed but has no file evidence with a path",
                    finding.id
                ));
            }
        }

        // (b) every department reference resolves to a registered department
        // id. A score/coverage row's department must additionally be present
        // in this report's own `departments` list — a registered id with no
        // corresponding department run would otherwise let a spurious score
        // or coverage entry inflate `overall` for a department this report
        // never actually ran.
        for finding in &self.findings {
            if !registry.contains(&finding.department) {
                return Err(format!(
                    "finding \"{}\" references unregistered department \"{}\"",
                    finding.id, finding.department
                ));
            }
        }
        for entry in &self.score_dashboard.entries {
            if !registry.contains(&entry.department) {
                return Err(format!(
                    "score entry references unregistered department \"{}\"",
                    entry.department
                ));
            }
            if !self
                .departments
                .iter()
                .any(|department| department.id == entry.department)
            {
                return Err(format!(
                    "score entry references department \"{}\" that is not present in this report's departments",
                    entry.department
                ));
            }
        }
        for row in &self.coverage_matrix {
            if !registry.contains(&row.department) {
                return Err(format!(
                    "coverage row references unregistered department \"{}\"",
                    row.department
                ));
            }
            if !self
                .departments
                .iter()
                .any(|department| department.id == row.department)
            {
                return Err(format!(
                    "coverage row references department \"{}\" that is not present in this report's departments",
                    row.department
                ));
            }
        }

        // (c) `self.departments` exactly matches the registry's department
        // ids: every registered department has a run in this report, and
        // every department run in this report names a registered department.
        for department in &registry.departments {
            if !self.departments.iter().any(|run| run.id == department.id) {
                return Err(format!(
                    "audit report is missing a department run for registered department \"{}\"",
                    department.id
                ));
            }
        }
        for department in &self.departments {
            if !registry.contains(&department.id) {
                return Err(format!(
                    "department run references unregistered department \"{}\"",
                    department.id
                ));
            }
        }

        // (d) a department run's status is consistent with the registry's
        // `enabled` flag: a disabled registry entry must be reported
        // disabled, and an enabled registry entry must be reported assessed
        // or not_assessed (never disabled). Every id here is registered —
        // rule (c) above already fails closed otherwise.
        for department in &self.departments {
            let Some(entry) = registry.get(&department.id) else {
                continue;
            };
            if !entry.enabled && department.status != DepartmentRunStatus::Disabled {
                return Err(format!(
                    "department \"{}\" is disabled in the registry but its run status is \"{}\", expected \"disabled\"",
                    department.id,
                    department.status.as_str()
                ));
            }
            if entry.enabled && department.status == DepartmentRunStatus::Disabled {
                return Err(format!(
                    "department \"{}\" is enabled in the registry but its run status is \"disabled\"",
                    department.id
                ));
            }
        }

        // (e) a not_assessed/disabled department carries a null score and a
        // not_assessed coverage row.
        for department in &self.departments {
            if !matches!(
                department.status,
                DepartmentRunStatus::NotAssessed | DepartmentRunStatus::Disabled
            ) {
                continue;
            }
            if let Some(entry) = self
                .score_dashboard
                .entries
                .iter()
                .find(|entry| entry.department == department.id)
            {
                if entry.score.is_some() {
                    return Err(format!(
                        "department \"{}\" is {} but its score entry is not null",
                        department.id,
                        department.status.as_str()
                    ));
                }
            }
            if let Some(row) = self
                .coverage_matrix
                .iter()
                .find(|row| row.department == department.id)
            {
                if row.coverage != Coverage::NotAssessed {
                    return Err(format!(
                        "department \"{}\" is {} but its coverage row is \"{}\", expected \"not_assessed\"",
                        department.id,
                        department.status.as_str(),
                        row.coverage.as_str()
                    ));
                }
            }
        }

        // (f) overall is the mean of non-null entry scores, rounded to 1
        // decimal, or null when nothing scored.
        let scored = self
            .score_dashboard
            .entries
            .iter()
            .filter_map(|entry| entry.score)
            .collect::<Vec<_>>();
        let recomputed = if scored.is_empty() {
            None
        } else {
            Some(round1(scored.iter().sum::<f64>() / scored.len() as f64))
        };
        if !nullable_f64_eq(recomputed, self.score_dashboard.overall) {
            return Err(format!(
                "score_dashboard.overall is {:?} but the recomputed mean of non-null entries is {:?}",
                self.score_dashboard.overall, recomputed
            ));
        }

        // (g) a perfect score with zero findings requires High coverage.
        for entry in &self.score_dashboard.entries {
            if !entry.score.is_some_and(is_perfect_score) {
                continue;
            }
            let finding_count = self
                .findings
                .iter()
                .filter(|finding| finding.department == entry.department)
                .count();
            if finding_count != 0 {
                continue;
            }
            let is_high = self
                .coverage_matrix
                .iter()
                .find(|row| row.department == entry.department)
                .is_some_and(|row| row.coverage == Coverage::High);
            if !is_high {
                return Err(format!(
                    "department \"{}\" scored 10.0 with zero findings but coverage is not \"high\"",
                    entry.department
                ));
            }
        }

        // (h) every report department has exactly one coverage row and at
        // most one score entry.
        for department in &self.departments {
            let coverage_count = self
                .coverage_matrix
                .iter()
                .filter(|row| row.department == department.id)
                .count();
            if coverage_count != 1 {
                return Err(format!(
                    "department \"{}\" has {coverage_count} coverage rows, expected exactly 1",
                    department.id
                ));
            }
            let score_count = self
                .score_dashboard
                .entries
                .iter()
                .filter(|entry| entry.department == department.id)
                .count();
            if score_count > 1 {
                return Err(format!(
                    "department \"{}\" has {score_count} score entries, expected at most 1",
                    department.id
                ));
            }
        }

        // (i) an assessed department must actually contribute to the health
        // score: a missing or null score entry would silently drop it from
        // the recomputed overall (rule (f)), and a not_assessed coverage row
        // would contradict the assessed status. Coverage-row existence
        // itself is rule (h)'s job.
        for department in &self.departments {
            if department.status != DepartmentRunStatus::Assessed {
                continue;
            }
            let has_score = self
                .score_dashboard
                .entries
                .iter()
                .any(|entry| entry.department == department.id && entry.score.is_some());
            if !has_score {
                return Err(format!(
                    "department \"{}\" is assessed but has no non-null score entry",
                    department.id
                ));
            }
            if let Some(row) = self
                .coverage_matrix
                .iter()
                .find(|row| row.department == department.id)
            {
                if row.coverage == Coverage::NotAssessed {
                    return Err(format!(
                        "department \"{}\" is assessed but its coverage row is \"not_assessed\"",
                        department.id
                    ));
                }
            }
        }

        Ok(())
    }

    /// Grounds file evidence in the tree it claims to cite. `validate()` above
    /// is filesystem-free — it can only check that a confirmed finding *has* a
    /// path, not that the path names a real file. A department is an agent, so
    /// an unresolvable or drifted citation is the expected failure mode, and
    /// without this pass a fabricated `path` validates green. Every `file`
    /// evidence entry must therefore be repo-relative, exist under `repo_root`,
    /// and carry a line range that fits the file it points at.
    pub(crate) fn validate_evidence_grounding(&self, repo_root: &Path) -> Result<(), String> {
        for finding in &self.findings {
            for entry in &finding.evidence {
                if entry.kind != EvidenceKind::File {
                    continue;
                }
                let Some(path) = entry.path.as_deref() else {
                    continue;
                };
                let relative =
                    repo_relative_path(path, &format!("finding \"{}\" evidence", finding.id))?;
                let resolved = repo_root.join(&relative);
                let contents = fs::read(&resolved).map_err(|error| {
                    format!(
                        "finding \"{}\" cites evidence that does not resolve under the repository: {relative} ({error})",
                        finding.id
                    )
                })?;
                let Some(line_start) = entry.line_start else {
                    continue;
                };
                let lines = contents.iter().filter(|byte| **byte == b'\n').count() as u64 + 1;
                let line_end = entry.line_end.unwrap_or(line_start);
                if line_end < line_start {
                    return Err(format!(
                        "finding \"{}\" cites {relative} with line_end {line_end} before line_start {line_start}",
                        finding.id
                    ));
                }
                if line_end > lines {
                    return Err(format!(
                        "finding \"{}\" cites {relative} lines {line_start}-{line_end} but the file has {lines} lines",
                        finding.id
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "validate_tests.rs"]
mod tests;
