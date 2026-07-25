use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde_json::{json, Map, Value};

#[path = "content_contract.rs"]
mod content_contract;

use content_contract::reject_duplicate_json_keys;

// ---------------------------------------------------------------------
// Generic JSON-object helpers. `closed_object` is this module's
// `additionalProperties: false`: every key must be declared required or
// optional, and every required key must be present.
// ---------------------------------------------------------------------

fn as_object<'a>(value: &'a Value, context: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{context} must be an object"))
}

fn closed_object<'a>(
    value: &'a Value,
    required: &[&str],
    optional: &[&str],
    context: &str,
) -> Result<&'a Map<String, Value>, String> {
    let object = as_object(value, context)?;
    for key in required {
        if !object.contains_key(*key) {
            return Err(format!("{context} is missing required field \"{key}\""));
        }
    }
    for key in object.keys() {
        let key = key.as_str();
        if !required.contains(&key) && !optional.contains(&key) {
            return Err(format!("{context} has an unrecognized field \"{key}\""));
        }
    }
    Ok(object)
}

fn required_str(object: &Map<String, Value>, key: &str, context: &str) -> Result<String, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{context}.{key} must be a non-empty string"))
}

fn optional_str(object: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("{key} must be a string when present")),
    }
}

fn required_nullable_string(
    object: &Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<Option<String>, String> {
    match object.get(key) {
        None => Err(format!("{context} is missing required field \"{key}\"")),
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("{context}.{key} must be a string or null")),
    }
}

fn required_bool(object: &Map<String, Value>, key: &str, context: &str) -> Result<bool, String> {
    object
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{context}.{key} must be a boolean"))
}

fn required_nullable_number(
    object: &Map<String, Value>,
    key: &str,
    min: f64,
    max: f64,
    context: &str,
) -> Result<Option<f64>, String> {
    match object.get(key) {
        None => Err(format!("{context} is missing required field \"{key}\"")),
        Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|number| *number >= min && *number <= max)
            .map(Some)
            .ok_or_else(|| format!("{context}.{key} must be a number between {min} and {max}")),
    }
}

fn optional_nullable_uint_min(
    object: &Map<String, Value>,
    key: &str,
    min: u64,
    context: &str,
) -> Result<Option<u64>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .filter(|number| *number >= min)
            .map(Some)
            .ok_or_else(|| format!("{context}.{key} must be an integer >= {min}")),
    }
}

fn required_string_array(
    object: &Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<Vec<String>, String> {
    let items = object
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{context}.{key} must be an array"))?;
    items
        .iter()
        .map(|item| {
            item.as_str()
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("{context}.{key} entries must be non-empty strings"))
        })
        .collect()
}

fn optional_string_array(
    object: &Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<Vec<String>, String> {
    match object.get(key) {
        None => Ok(Vec::new()),
        Some(value) => {
            let items = value
                .as_array()
                .ok_or_else(|| format!("{context}.{key} must be an array"))?;
            items
                .iter()
                .map(|item| {
                    item.as_str()
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .ok_or_else(|| format!("{context}.{key} entries must be non-empty strings"))
                })
                .collect()
        }
    }
}

fn required_enum<T>(
    object: &Map<String, Value>,
    key: &str,
    context: &str,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<T, String> {
    let raw = object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{context}.{key} must be a string"))?;
    parse(raw).ok_or_else(|| format!("{context}.{key} has an unrecognized value \"{raw}\""))
}

fn optional_nullable_enum<T>(
    object: &Map<String, Value>,
    key: &str,
    context: &str,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<Option<T>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => parse(raw)
            .map(Some)
            .ok_or_else(|| format!("{context}.{key} has an unrecognized value \"{raw}\"")),
        Some(_) => Err(format!("{context}.{key} must be a string or null")),
    }
}

/// Matches the schema pattern `^[a-z0-9-]+-[0-9]{3}$`: a lowercase
/// alnum/hyphen prefix followed by a literal hyphen and exactly 3 digits.
fn is_finding_id(value: &str) -> bool {
    if value.len() < 5 {
        return false;
    }
    let (prefix, suffix) = value.split_at(value.len() - 4);
    !prefix.is_empty()
        && prefix
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && suffix.starts_with('-')
        && suffix.as_bytes()[1..].iter().all(u8::is_ascii_digit)
}

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

// ---------------------------------------------------------------------
// Enums. `as_str` always returns the schema's lowercase wire value;
// `parse` is its exact inverse.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Info => "info",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "critical" => Some(Self::Critical),
            "high" => Some(Self::High),
            "medium" => Some(Self::Medium),
            "low" => Some(Self::Low),
            "info" => Some(Self::Info),
            _ => None,
        }
    }

    /// Ascending rank for "severity desc" sorts: 0 is the most severe.
    fn rank(self) -> u8 {
        match self {
            Self::Critical => 0,
            Self::High => 1,
            Self::Medium => 2,
            Self::Low => 3,
            Self::Info => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Confidence {
    High,
    Medium,
    Low,
}

impl Confidence {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "high" => Some(Self::High),
            "medium" => Some(Self::Medium),
            "low" => Some(Self::Low),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DepartmentRunStatus {
    Assessed,
    NotAssessed,
    Disabled,
}

impl DepartmentRunStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Assessed => "assessed",
            Self::NotAssessed => "not_assessed",
            Self::Disabled => "disabled",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "assessed" => Some(Self::Assessed),
            "not_assessed" => Some(Self::NotAssessed),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Applicable {
    Yes,
    No,
    Unknown,
}

impl Applicable {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "yes" => Some(Self::Yes),
            "no" => Some(Self::No),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindingStatus {
    Confirmed,
    Suspected,
}

impl FindingStatus {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "confirmed" => Some(Self::Confirmed),
            "suspected" => Some(Self::Suspected),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EstimatedEffort {
    Minutes,
    Hours,
    Days,
}

impl EstimatedEffort {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "minutes" => Some(Self::Minutes),
            "hours" => Some(Self::Hours),
            "days" => Some(Self::Days),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvidenceKind {
    File,
    ModalitySignal,
    Command,
    ManualRead,
}

impl EvidenceKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "file" => Some(Self::File),
            "modality_signal" => Some(Self::ModalitySignal),
            "command" => Some(Self::Command),
            "manual_read" => Some(Self::ManualRead),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Modality {
    Xray,
    Anatomy,
    Ct,
    Mri,
    Pet,
    Chart,
    Governance,
}

impl Modality {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "xray" => Some(Self::Xray),
            "anatomy" => Some(Self::Anatomy),
            "ct" => Some(Self::Ct),
            "mri" => Some(Self::Mri),
            "pet" => Some(Self::Pet),
            "chart" => Some(Self::Chart),
            "governance" => Some(Self::Governance),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Coverage {
    High,
    Medium,
    Low,
    NotAssessed,
}

impl Coverage {
    fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::NotAssessed => "not_assessed",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "high" => Some(Self::High),
            "medium" => Some(Self::Medium),
            "low" => Some(Self::Low),
            "not_assessed" => Some(Self::NotAssessed),
            _ => None,
        }
    }
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

#[derive(Debug)]
pub(crate) struct AuditReport {
    pub(crate) generated_at: Option<String>,
    pub(crate) repo: String,
    pub(crate) departments: Vec<DepartmentRun>,
    pub(crate) findings: Vec<Finding>,
    pub(crate) score_dashboard: ScoreDashboard,
    pub(crate) coverage_matrix: Vec<CoverageRow>,
}

impl AuditReport {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, String> {
        let text = std::str::from_utf8(bytes)
            .map_err(|error| format!("audit report is not UTF-8: {error}"))?;
        reject_duplicate_json_keys(text)?;
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
            &[],
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
        Ok(Self {
            generated_at: required_nullable_string(object, "generatedAt", "audit report")?,
            repo: required_str(object, "repo", "audit report")?,
            departments,
            findings,
            score_dashboard,
            coverage_matrix,
        })
    }

    /// Fail-closed invariants beyond what the JSON Schema can express.
    /// Each branch below is one distinct, independently testable rule.
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

        // (b) every department reference resolves to a registered department id.
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
        }
        for row in &self.coverage_matrix {
            if !registry.contains(&row.department) {
                return Err(format!(
                    "coverage row references unregistered department \"{}\"",
                    row.department
                ));
            }
        }

        // (c) a not_assessed/disabled department carries a null score and a
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

        // (d) overall is the mean of non-null entry scores, rounded to 1
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

        // (e) a perfect score with zero findings requires High coverage.
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

        // (f) every report department has exactly one coverage row and at
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

        Ok(())
    }

    /// The compact block embedded in `hospital-report.json.audit`. The rich
    /// score/coverage/findings detail lives in `render_markdown_section`
    /// instead, since the hospital schema's `audit` property is a pointer
    /// plus counters, not the full report.
    pub(crate) fn summary(&self, artifact: &str) -> AuditSummary {
        let findings_total = self.findings.len() as u64;
        let by_severity = if findings_total == 0 {
            None
        } else {
            let count_of = |severity: Severity| {
                let count = self
                    .findings
                    .iter()
                    .filter(|finding| finding.severity == severity)
                    .count() as u64;
                (count > 0).then_some(count)
            };
            Some(BySeverity {
                critical: count_of(Severity::Critical),
                high: count_of(Severity::High),
                medium: count_of(Severity::Medium),
                low: count_of(Severity::Low),
                info: count_of(Severity::Info),
            })
        };
        AuditSummary {
            status: AuditSummaryStatus::Present,
            artifact: Some(artifact.to_string()),
            overall: self.score_dashboard.overall,
            findings_total: Some(findings_total),
            by_severity,
        }
    }
}

// ---------------------------------------------------------------------
// The compact `hospital-report.json.audit` block.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuditSummaryStatus {
    Absent,
    Present,
}

impl AuditSummaryStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Present => "present",
        }
    }
}

pub(crate) struct BySeverity {
    pub(crate) critical: Option<u64>,
    pub(crate) high: Option<u64>,
    pub(crate) medium: Option<u64>,
    pub(crate) low: Option<u64>,
    pub(crate) info: Option<u64>,
}

impl BySeverity {
    fn to_value(&self) -> Value {
        let mut object = Map::new();
        for (key, count) in [
            ("critical", self.critical),
            ("high", self.high),
            ("medium", self.medium),
            ("low", self.low),
            ("info", self.info),
        ] {
            if let Some(count) = count {
                object.insert(key.to_string(), Value::from(count));
            }
        }
        Value::Object(object)
    }
}

pub(crate) struct AuditSummary {
    pub(crate) status: AuditSummaryStatus,
    pub(crate) artifact: Option<String>,
    pub(crate) overall: Option<f64>,
    pub(crate) findings_total: Option<u64>,
    pub(crate) by_severity: Option<BySeverity>,
}

impl AuditSummary {
    pub(crate) fn to_value(&self) -> Value {
        json!({
            "status": self.status.as_str(),
            "artifact": self.artifact,
            "overall": self.overall,
            "findings_total": self.findings_total,
            "by_severity": self.by_severity.as_ref().map(BySeverity::to_value),
        })
    }
}

fn format_score(score: Option<f64>) -> String {
    match score {
        Some(value) => format!("{value:.1}"),
        None => "n/a".to_string(),
    }
}

/// The rich `## Audit` section for `hospital.md`: score table, coverage
/// matrix, and up to 10 findings sorted by severity (most severe first).
pub(crate) fn render_markdown_section(report: &AuditReport) -> String {
    let mut section = format!(
        "\n## Audit\n\nOverall: {}\n\n### Score Dashboard\n\n| Department | Score | Justification |\n| --- | --- | --- |\n",
        format_score(report.score_dashboard.overall)
    );
    for entry in &report.score_dashboard.entries {
        section.push_str(&format!(
            "| {} | {} | {} |\n",
            entry.department,
            format_score(entry.score),
            entry.justification
        ));
    }
    section.push_str(
        "\n### Coverage Matrix\n\n| Department | Coverage | Inspected Evidence | Exclusions |\n| --- | --- | --- | --- |\n",
    );
    for row in &report.coverage_matrix {
        section.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            row.department,
            row.coverage.as_str(),
            row.inspected_evidence.join(", "),
            row.exclusions.join(", ")
        ));
    }
    section.push_str("\n### Top Findings\n\n");
    let mut findings = report.findings.iter().collect::<Vec<_>>();
    findings.sort_by_key(|finding| finding.severity.rank());
    if findings.is_empty() {
        section.push_str("- none\n");
    } else {
        for finding in findings.into_iter().take(10) {
            section.push_str(&format!(
                "- {} | {} | {}\n",
                finding.severity.as_str(),
                finding.id,
                finding.title
            ));
        }
    }
    section
}

// ---------------------------------------------------------------------
// orchestration/audit/departments.v1.json registry
// ---------------------------------------------------------------------

pub(crate) struct RubricPaths {
    pub(crate) severity: String,
    pub(crate) confidence: String,
    pub(crate) evidence: String,
    pub(crate) coverage: String,
    pub(crate) scoring: String,
}

impl RubricPaths {
    fn from_value(value: &Value) -> Result<Self, String> {
        let object = closed_object(
            value,
            &["severity", "confidence", "evidence", "coverage", "scoring"],
            &[],
            "rubrics",
        )?;
        Ok(Self {
            severity: required_str(object, "severity", "rubrics")?,
            confidence: required_str(object, "confidence", "rubrics")?,
            evidence: required_str(object, "evidence", "rubrics")?,
            coverage: required_str(object, "coverage", "rubrics")?,
            scoring: required_str(object, "scoring", "rubrics")?,
        })
    }

    fn paths(&self) -> [(&'static str, &str); 5] {
        [
            ("severity", self.severity.as_str()),
            ("confidence", self.confidence.as_str()),
            ("evidence", self.evidence.as_str()),
            ("coverage", self.coverage.as_str()),
            ("scoring", self.scoring.as_str()),
        ]
    }
}

pub(crate) struct DepartmentEntry {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) enabled: bool,
    pub(crate) prompt: String,
    pub(crate) consumes: Vec<String>,
    pub(crate) applicability_check: String,
    pub(crate) tracking_issue: String,
}

impl DepartmentEntry {
    fn from_value(value: &Value) -> Result<Self, String> {
        let object = closed_object(
            value,
            &[
                "id",
                "title",
                "enabled",
                "prompt",
                "consumes",
                "applicabilityCheck",
                "trackingIssue",
            ],
            &[],
            "department entry",
        )?;
        Ok(Self {
            id: required_str(object, "id", "department entry")?,
            title: required_str(object, "title", "department entry")?,
            enabled: required_bool(object, "enabled", "department entry")?,
            prompt: required_str(object, "prompt", "department entry")?,
            consumes: required_string_array(object, "consumes", "department entry")?,
            applicability_check: required_str(object, "applicabilityCheck", "department entry")?,
            tracking_issue: required_str(object, "trackingIssue", "department entry")?,
        })
    }
}

pub(crate) struct DepartmentRegistry {
    pub(crate) catalog_version: String,
    pub(crate) rubrics: RubricPaths,
    pub(crate) finding_contract: String,
    pub(crate) departments: Vec<DepartmentEntry>,
}

impl DepartmentRegistry {
    pub(crate) fn load(repo_root: &Path) -> Result<Self, String> {
        let path = repo_root.join("orchestration/audit/departments.v1.json");
        let bytes = fs::read(&path)
            .map_err(|error| format!("read department registry {}: {error}", path.display()))?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|error| format!("department registry is not UTF-8: {error}"))?;
        reject_duplicate_json_keys(text)?;
        let value: Value = serde_json::from_str(text)
            .map_err(|error| format!("department registry is not JSON: {error}"))?;
        Self::from_value(&value)
    }

    fn from_value(value: &Value) -> Result<Self, String> {
        let object = closed_object(
            value,
            &[
                "schema",
                "catalogVersion",
                "rubrics",
                "findingContract",
                "departments",
            ],
            &[],
            "department registry",
        )?;
        let schema = required_str(object, "schema", "department registry")?;
        if schema != "code-intel-audit-departments.v1" {
            return Err(
                "department registry schema must equal \"code-intel-audit-departments.v1\""
                    .to_string(),
            );
        }
        let rubrics = object
            .get("rubrics")
            .ok_or_else(|| "department registry is missing required field \"rubrics\"".to_string())
            .and_then(RubricPaths::from_value)?;
        let departments = object
            .get("departments")
            .and_then(Value::as_array)
            .ok_or_else(|| "department registry.departments must be an array".to_string())?
            .iter()
            .map(DepartmentEntry::from_value)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            catalog_version: required_str(object, "catalogVersion", "department registry")?,
            rubrics,
            finding_contract: required_str(object, "findingContract", "department registry")?,
            departments,
        })
    }

    pub(crate) fn contains(&self, id: &str) -> bool {
        self.departments
            .iter()
            .any(|department| department.id == id)
    }

    /// Registry-level invariants: unique department ids, rubric files that
    /// actually exist, and a prompt file for every *enabled* department.
    /// `enabled: false` departments may point at a prompt path that does
    /// not exist yet — that file is the department ticket's job.
    pub(crate) fn validate(&self, repo_root: &Path) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for department in &self.departments {
            if !seen.insert(department.id.as_str()) {
                return Err(format!(
                    "duplicate department id \"{}\" in registry",
                    department.id
                ));
            }
        }
        for (label, relative) in self.rubrics.paths() {
            if !repo_root.join(relative).is_file() {
                return Err(format!("rubrics.{label} file does not exist: {relative}"));
            }
        }
        for department in &self.departments {
            if department.enabled && !repo_root.join(&department.prompt).is_file() {
                return Err(format!(
                    "department \"{}\" is enabled but its prompt file does not exist: {}",
                    department.id, department.prompt
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn fixture_bytes() -> Vec<u8> {
        fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/audit/audit-report.v1.example.json"),
        )
        .unwrap()
    }

    fn fixture_value() -> Value {
        serde_json::from_slice(&fixture_bytes()).unwrap()
    }

    fn registry() -> DepartmentRegistry {
        DepartmentRegistry::load(&repo_root()).unwrap()
    }

    #[test]
    fn parses_and_validates_the_example_fixture() {
        let report = AuditReport::parse(&fixture_bytes()).unwrap();
        report.validate(&registry()).unwrap();
    }

    #[test]
    fn rejects_unknown_top_level_field() {
        let mut value = fixture_value();
        value["bogus"] = json!(true);
        let error = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert!(error.contains("unrecognized field"), "{error}");
    }

    #[test]
    fn rejects_finding_with_no_evidence_entries() {
        let mut value = fixture_value();
        value["findings"][0]["evidence"] = json!([]);
        let error = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert!(error.contains("at least one entry"), "{error}");
    }

    #[test]
    fn rejects_confirmed_finding_without_file_evidence() {
        let mut value = fixture_value();
        value["findings"][0]["evidence"] =
            json!([{"kind": "command", "source": "sentrux check --repo ."}]);
        let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        let error = report.validate(&registry()).unwrap_err();
        assert!(error.contains("no file evidence"), "{error}");
    }

    #[test]
    fn rejects_unregistered_department_reference() {
        let mut value = fixture_value();
        value["findings"][0]["department"] = json!("not-a-real-department");
        let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        let error = report.validate(&registry()).unwrap_err();
        assert!(error.contains("unregistered department"), "{error}");
    }

    #[test]
    fn rejects_not_assessed_department_with_present_coverage() {
        let mut value = fixture_value();
        let index = value["coverage_matrix"]
            .as_array()
            .unwrap()
            .iter()
            .position(|row| row["department"] == "ai-safety")
            .unwrap();
        value["coverage_matrix"][index]["coverage"] = json!("high");
        let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        let error = report.validate(&registry()).unwrap_err();
        assert!(error.contains("expected \"not_assessed\""), "{error}");
    }

    #[test]
    fn rejects_not_assessed_department_with_a_non_null_score() {
        let mut value = fixture_value();
        let index = value["score_dashboard"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .position(|entry| entry["department"] == "ai-safety")
            .unwrap();
        value["score_dashboard"]["entries"][index]["score"] = json!(5.0);
        let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        let error = report.validate(&registry()).unwrap_err();
        assert!(error.contains("score entry is not null"), "{error}");
    }

    #[test]
    fn rejects_overall_score_mismatch() {
        let mut value = fixture_value();
        value["score_dashboard"]["overall"] = json!(5.0);
        let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        let error = report.validate(&registry()).unwrap_err();
        assert!(error.contains("recomputed mean"), "{error}");
    }

    #[test]
    fn rejects_perfect_score_with_zero_findings_and_non_high_coverage() {
        let mut value = fixture_value();
        value["findings"] = json!([]);
        value["score_dashboard"]["entries"][0]["score"] = json!(10.0);
        value["score_dashboard"]["overall"] = json!(10.0);
        value["coverage_matrix"][0]["coverage"] = json!("medium");
        let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        let error = report.validate(&registry()).unwrap_err();
        assert!(error.contains("scored 10.0 with zero findings"), "{error}");
    }

    #[test]
    fn rejects_duplicate_coverage_row_for_a_department() {
        let mut value = fixture_value();
        let duplicate = value["coverage_matrix"][0].clone();
        value["coverage_matrix"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        let error = report.validate(&registry()).unwrap_err();
        assert!(
            error.contains("coverage rows, expected exactly 1"),
            "{error}"
        );
    }

    #[test]
    fn rejects_duplicate_score_entry_for_a_department() {
        let mut value = fixture_value();
        let duplicate = value["score_dashboard"]["entries"][0].clone();
        value["score_dashboard"]["entries"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        let report = AuditReport::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        let error = report.validate(&registry()).unwrap_err();
        assert!(
            error.contains("score entries, expected at most 1"),
            "{error}"
        );
    }

    #[test]
    fn summary_reflects_findings_total_and_severity_counts() {
        let report = AuditReport::parse(&fixture_bytes()).unwrap();
        let summary = report.summary("audit-report.json");
        assert_eq!(summary.status, AuditSummaryStatus::Present);
        assert_eq!(summary.artifact.as_deref(), Some("audit-report.json"));
        assert_eq!(summary.findings_total, Some(1));
        let by_severity = summary.by_severity.unwrap();
        assert_eq!(by_severity.medium, Some(1));
        assert_eq!(by_severity.critical, None);
    }

    #[test]
    fn render_markdown_section_lists_score_coverage_and_top_findings() {
        let report = AuditReport::parse(&fixture_bytes()).unwrap();
        let markdown = render_markdown_section(&report);
        assert!(markdown.contains("## Audit"));
        assert!(markdown.contains("| security |"));
        assert!(markdown.contains("### Coverage Matrix"));
        assert!(markdown.contains("medium | security-001 |"));
    }

    #[test]
    fn registry_loads_and_validates_the_real_departments_file() {
        let registry = registry();
        assert_eq!(registry.departments.len(), 3);
        registry.validate(&repo_root()).unwrap();
    }

    #[test]
    fn registry_rejects_duplicate_department_ids() {
        let value = json!({
            "schema": "code-intel-audit-departments.v1",
            "catalogVersion": "1.0.0",
            "rubrics": {
                "severity": "a", "confidence": "b", "evidence": "c",
                "coverage": "d", "scoring": "e"
            },
            "findingContract": "docs/audit-report.md",
            "departments": [
                {"id":"security","title":"Security","enabled":false,"prompt":"p1","consumes":[],"applicabilityCheck":"always","trackingIssue":"t"},
                {"id":"security","title":"Security Again","enabled":false,"prompt":"p2","consumes":[],"applicabilityCheck":"always","trackingIssue":"t"}
            ]
        });
        let registry = DepartmentRegistry::from_value(&value).unwrap();
        let error = registry.validate(&repo_root()).unwrap_err();
        assert!(error.contains("duplicate department id"), "{error}");
    }

    #[test]
    fn registry_rejects_missing_rubric_file() {
        let value = json!({
            "schema": "code-intel-audit-departments.v1",
            "catalogVersion": "1.0.0",
            "rubrics": {
                "severity": "orchestration/audit/rubrics/does-not-exist.md",
                "confidence": "orchestration/audit/rubrics/confidence.md",
                "evidence": "orchestration/audit/rubrics/evidence.md",
                "coverage": "orchestration/audit/rubrics/coverage.md",
                "scoring": "orchestration/audit/rubrics/scoring.md"
            },
            "findingContract": "docs/audit-report.md",
            "departments": []
        });
        let registry = DepartmentRegistry::from_value(&value).unwrap();
        let error = registry.validate(&repo_root()).unwrap_err();
        assert!(error.contains("does not exist"), "{error}");
    }

    #[test]
    fn registry_requires_prompt_file_when_enabled() {
        let value = json!({
            "schema": "code-intel-audit-departments.v1",
            "catalogVersion": "1.0.0",
            "rubrics": {
                "severity": "orchestration/audit/rubrics/severity.md",
                "confidence": "orchestration/audit/rubrics/confidence.md",
                "evidence": "orchestration/audit/rubrics/evidence.md",
                "coverage": "orchestration/audit/rubrics/coverage.md",
                "scoring": "orchestration/audit/rubrics/scoring.md"
            },
            "findingContract": "docs/audit-report.md",
            "departments": [
                {"id":"synthetic","title":"Synthetic","enabled":true,"prompt":"orchestration/audit/prompts/does-not-exist.md","consumes":[],"applicabilityCheck":"always","trackingIssue":"t"}
            ]
        });
        let registry = DepartmentRegistry::from_value(&value).unwrap();
        let error = registry.validate(&repo_root()).unwrap_err();
        assert!(
            error.contains("enabled but its prompt file does not exist"),
            "{error}"
        );
    }

    #[test]
    fn registry_allows_a_disabled_department_with_a_missing_prompt_file() {
        let value = json!({
            "schema": "code-intel-audit-departments.v1",
            "catalogVersion": "1.0.0",
            "rubrics": {
                "severity": "orchestration/audit/rubrics/severity.md",
                "confidence": "orchestration/audit/rubrics/confidence.md",
                "evidence": "orchestration/audit/rubrics/evidence.md",
                "coverage": "orchestration/audit/rubrics/coverage.md",
                "scoring": "orchestration/audit/rubrics/scoring.md"
            },
            "findingContract": "docs/audit-report.md",
            "departments": [
                {"id":"synthetic","title":"Synthetic","enabled":false,"prompt":"orchestration/audit/prompts/does-not-exist.md","consumes":[],"applicabilityCheck":"always","trackingIssue":"t"}
            ]
        });
        let registry = DepartmentRegistry::from_value(&value).unwrap();
        registry.validate(&repo_root()).unwrap();
    }

    #[test]
    fn finding_id_pattern_accepts_and_rejects_expected_shapes() {
        assert!(is_finding_id("security-001"));
        assert!(is_finding_id("ai-safety-042"));
        assert!(!is_finding_id("security-1"));
        assert!(!is_finding_id("Security-001"));
        assert!(!is_finding_id("-001"));
        assert!(!is_finding_id("security001"));
    }
}
