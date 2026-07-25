use serde_json::Value;

use super::{
    enums::{
        Applicable, Confidence, Coverage, DepartmentRunStatus, EstimatedEffort, EvidenceKind,
        FindingStatus, Modality, ScopeKind, Severity,
    },
    json_helpers::{
        closed_object, optional_nullable_enum, optional_nullable_uint_min, optional_object,
        optional_str, optional_string_array, required_bool, required_enum,
        required_nullable_number, required_nullable_string, required_str, required_string_array,
    },
};

/// Matches the schema pattern `^[a-z0-9-]+-[0-9]{3}$`: a lowercase
/// alnum/hyphen prefix followed by a literal hyphen and exactly 3 digits.
///
/// Operates on `as_bytes()` end-to-end rather than `str::split_at`: splitting
/// a `&str` at a byte offset panics if that offset falls inside a multi-byte
/// UTF-8 sequence (e.g. non-ASCII input like "日日"), but splitting a `&[u8]`
/// never has a char-boundary constraint, so this is panic-free for any input.
pub(super) fn is_finding_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 5 {
        return false;
    }
    let (prefix, suffix) = bytes.split_at(bytes.len() - 4);
    !prefix.is_empty()
        && prefix
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
        && suffix[0] == b'-'
        && suffix[1..].iter().all(u8::is_ascii_digit)
}

// ---------------------------------------------------------------------
// code-intel-audit-report.v1 types
// ---------------------------------------------------------------------

#[derive(Debug)]
pub(crate) struct EvidenceRef {
    pub(crate) kind: EvidenceKind,
    pub(crate) source: String,
    pub(crate) path: Option<String>,
    pub(crate) line_start: Option<u64>,
    pub(crate) line_end: Option<u64>,
    pub(crate) modality: Option<Modality>,
    pub(crate) note: Option<String>,
}

impl EvidenceRef {
    fn from_value(value: &Value) -> Result<Self, String> {
        let object = closed_object(
            value,
            &["kind", "source"],
            &["path", "line_start", "line_end", "modality", "note"],
            "evidence entry",
        )?;
        Ok(Self {
            kind: required_enum(object, "kind", "evidence entry", EvidenceKind::parse)?,
            source: required_str(object, "source", "evidence entry")?,
            path: optional_str(object, "path")?,
            line_start: optional_nullable_uint_min(object, "line_start", 1, "evidence entry")?,
            line_end: optional_nullable_uint_min(object, "line_end", 1, "evidence entry")?,
            modality: optional_nullable_enum(
                object,
                "modality",
                "evidence entry",
                Modality::parse,
            )?,
            note: optional_str(object, "note")?,
        })
    }
}

#[derive(Debug)]
pub(crate) struct Applicability {
    pub(crate) applicable: Applicable,
    pub(crate) reason: String,
    pub(crate) surface_evidence: Vec<String>,
}

impl Applicability {
    fn from_value(value: &Value) -> Result<Self, String> {
        let object = closed_object(
            value,
            &["applicable", "reason"],
            &["surface_evidence"],
            "applicability",
        )?;
        Ok(Self {
            applicable: required_enum(object, "applicable", "applicability", Applicable::parse)?,
            reason: required_str(object, "reason", "applicability")?,
            surface_evidence: optional_string_array(object, "surface_evidence", "applicability")?,
        })
    }
}

#[derive(Debug)]
pub(crate) struct DepartmentRun {
    pub(crate) id: String,
    pub(crate) status: DepartmentRunStatus,
    pub(crate) applicability: Applicability,
}

impl DepartmentRun {
    fn from_value(value: &Value) -> Result<Self, String> {
        let object = closed_object(
            value,
            &["id", "status", "applicability"],
            &[],
            "department run",
        )?;
        let applicability = object
            .get("applicability")
            .ok_or_else(|| "department run is missing required field \"applicability\"".to_string())
            .and_then(Applicability::from_value)?;
        Ok(Self {
            id: required_str(object, "id", "department run")?,
            status: required_enum(
                object,
                "status",
                "department run",
                DepartmentRunStatus::parse,
            )?,
            applicability,
        })
    }
}

#[derive(Debug)]
pub(crate) struct Finding {
    pub(crate) id: String,
    pub(crate) department: String,
    pub(crate) title: String,
    pub(crate) severity: Severity,
    pub(crate) confidence: Confidence,
    pub(crate) status: FindingStatus,
    pub(crate) affected_area: Option<String>,
    pub(crate) evidence: Vec<EvidenceRef>,
    pub(crate) problem: String,
    pub(crate) failure_scenario: String,
    pub(crate) minimal_fix: String,
    pub(crate) long_term_fix: Option<String>,
    pub(crate) regression_test: String,
    pub(crate) estimated_effort: EstimatedEffort,
    pub(crate) redacted: bool,
}

impl Finding {
    fn from_value(value: &Value) -> Result<Self, String> {
        let object = closed_object(
            value,
            &[
                "id",
                "department",
                "title",
                "severity",
                "confidence",
                "status",
                "evidence",
                "problem",
                "failure_scenario",
                "minimal_fix",
                "regression_test",
                "estimated_effort",
                "redacted",
            ],
            &["affected_area", "long_term_fix"],
            "finding",
        )?;
        let id = required_str(object, "id", "finding")?;
        if !is_finding_id(&id) {
            return Err(format!(
                "finding.id \"{id}\" does not match pattern ^[a-z0-9-]+-[0-9]{{3}}$"
            ));
        }
        let evidence_values = object
            .get("evidence")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("finding \"{id}\" evidence must be an array"))?;
        if evidence_values.is_empty() {
            return Err(format!(
                "finding \"{id}\" evidence must contain at least one entry"
            ));
        }
        let evidence = evidence_values
            .iter()
            .map(EvidenceRef::from_value)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            department: required_str(object, "department", "finding")?,
            title: required_str(object, "title", "finding")?,
            severity: required_enum(object, "severity", "finding", Severity::parse)?,
            confidence: required_enum(object, "confidence", "finding", Confidence::parse)?,
            status: required_enum(object, "status", "finding", FindingStatus::parse)?,
            affected_area: optional_str(object, "affected_area")?,
            evidence,
            problem: required_str(object, "problem", "finding")?,
            failure_scenario: required_str(object, "failure_scenario", "finding")?,
            minimal_fix: required_str(object, "minimal_fix", "finding")?,
            long_term_fix: optional_str(object, "long_term_fix")?,
            regression_test: required_str(object, "regression_test", "finding")?,
            estimated_effort: required_enum(
                object,
                "estimated_effort",
                "finding",
                EstimatedEffort::parse,
            )?,
            redacted: required_bool(object, "redacted", "finding")?,
            id,
        })
    }
}

#[derive(Debug)]
pub(crate) struct ScoreEntry {
    pub(crate) department: String,
    pub(crate) score: Option<f64>,
    pub(crate) justification: String,
}

impl ScoreEntry {
    fn from_value(value: &Value) -> Result<Self, String> {
        let object = closed_object(
            value,
            &["department", "score", "justification"],
            &[],
            "score entry",
        )?;
        Ok(Self {
            department: required_str(object, "department", "score entry")?,
            score: required_nullable_number(object, "score", 0.0, 10.0, "score entry")?,
            justification: required_str(object, "justification", "score entry")?,
        })
    }
}

#[derive(Debug)]
pub(crate) struct ScoreDashboard {
    pub(crate) entries: Vec<ScoreEntry>,
    pub(crate) overall: Option<f64>,
}

impl ScoreDashboard {
    fn from_value(value: &Value) -> Result<Self, String> {
        let object = closed_object(value, &["entries", "overall"], &[], "score dashboard")?;
        let entries = object
            .get("entries")
            .and_then(Value::as_array)
            .ok_or_else(|| "score_dashboard.entries must be an array".to_string())?
            .iter()
            .map(ScoreEntry::from_value)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            overall: required_nullable_number(object, "overall", 0.0, 10.0, "score dashboard")?,
            entries,
        })
    }
}

#[derive(Debug)]
pub(crate) struct CoverageRow {
    pub(crate) department: String,
    pub(crate) coverage: Coverage,
    pub(crate) inspected_evidence: Vec<String>,
    pub(crate) exclusions: Vec<String>,
}

impl CoverageRow {
    fn from_value(value: &Value) -> Result<Self, String> {
        let object = closed_object(
            value,
            &["department", "coverage", "inspected_evidence", "exclusions"],
            &[],
            "coverage row",
        )?;
        Ok(Self {
            department: required_str(object, "department", "coverage row")?,
            coverage: required_enum(object, "coverage", "coverage row", Coverage::parse)?,
            inspected_evidence: required_string_array(
                object,
                "inspected_evidence",
                "coverage row",
            )?,
            exclusions: required_string_array(object, "exclusions", "coverage row")?,
        })
    }
}

/// The optional incremental-audit scope block (`docs/audit-report.md`'s
/// incremental audits section). `since`/`files` are plain optional keys, not
/// required-nullable like `generatedAt`: a `full` scope (or an absent one)
/// legitimately never carries them. `AuditReport::validate()` rule (j) is
/// what actually requires them to be present and non-empty when
/// `kind == diff` — the schema only requires `kind`.
#[derive(Debug)]
pub(crate) struct Scope {
    pub(crate) kind: ScopeKind,
    pub(crate) since: Option<String>,
    pub(crate) files: Vec<String>,
}

impl Scope {
    fn from_value(value: &Value) -> Result<Self, String> {
        let object = closed_object(value, &["kind"], &["since", "files"], "scope")?;
        Ok(Self {
            kind: required_enum(object, "kind", "scope", ScopeKind::parse)?,
            since: optional_str(object, "since")?,
            files: optional_string_array(object, "files", "scope")?,
        })
    }
}

#[derive(Debug)]
pub(crate) struct AuditReport {
    pub(crate) generated_at: Option<String>,
    pub(crate) repo: String,
    pub(crate) departments: Vec<DepartmentRun>,
    pub(crate) findings: Vec<Finding>,
    pub(crate) score_dashboard: ScoreDashboard,
    pub(crate) coverage_matrix: Vec<CoverageRow>,
    pub(crate) scope: Option<Scope>,
}

impl AuditReport {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, String> {
        let text = std::str::from_utf8(bytes)
            .map_err(|error| format!("audit report is not UTF-8: {error}"))?;
        super::content_contract::reject_duplicate_json_keys(text)?;
        let value: Value = serde_json::from_str(text)
            .map_err(|error| format!("audit report is not JSON: {error}"))?;
        Self::from_value(&value)
    }

    fn from_value(value: &Value) -> Result<Self, String> {
        let object = closed_object(
            value,
            &[
                "schema",
                "generatedAt",
                "repo",
                "rubric_version",
                "departments",
                "findings",
                "score_dashboard",
                "coverage_matrix",
            ],
            &["scope"],
            "audit report",
        )?;
        let schema = required_str(object, "schema", "audit report")?;
        if schema != "code-intel-audit-report.v1" {
            return Err(
                "audit report schema must equal \"code-intel-audit-report.v1\"".to_string(),
            );
        }
        let rubric_version = required_str(object, "rubric_version", "audit report")?;
        if rubric_version != "v1" {
            return Err("audit report rubric_version must equal \"v1\"".to_string());
        }
        let departments = object
            .get("departments")
            .and_then(Value::as_array)
            .ok_or_else(|| "audit report.departments must be an array".to_string())?
            .iter()
            .map(DepartmentRun::from_value)
            .collect::<Result<Vec<_>, _>>()?;
        let findings = object
            .get("findings")
            .and_then(Value::as_array)
            .ok_or_else(|| "audit report.findings must be an array".to_string())?
            .iter()
            .map(Finding::from_value)
            .collect::<Result<Vec<_>, _>>()?;
        let score_dashboard = object
            .get("score_dashboard")
            .ok_or_else(|| "audit report is missing required field \"score_dashboard\"".to_string())
            .and_then(ScoreDashboard::from_value)?;
        let coverage_matrix = object
            .get("coverage_matrix")
            .and_then(Value::as_array)
            .ok_or_else(|| "audit report.coverage_matrix must be an array".to_string())?
            .iter()
            .map(CoverageRow::from_value)
            .collect::<Result<Vec<_>, _>>()?;
        let scope = optional_object(object, "scope")?
            .map(Scope::from_value)
            .transpose()?;
        Ok(Self {
            generated_at: required_nullable_string(object, "generatedAt", "audit report")?,
            repo: required_str(object, "repo", "audit report")?,
            departments,
            findings,
            score_dashboard,
            coverage_matrix,
            scope,
        })
    }
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
