use std::{collections::BTreeSet, fs, path::Path};

use serde_json::Value;

use super::enums::Modality;
use super::json_helpers::{closed_object, required_bool, required_str, required_string_array};

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
        let id = required_str(object, "id", "department entry")?;
        let consumes = required_string_array(object, "consumes", "department entry")?;
        for modality in &consumes {
            if Modality::parse(modality).is_none() {
                return Err(format!(
                    "department \"{id}\" consumes unknown modality \"{modality}\""
                ));
            }
        }
        Ok(Self {
            id,
            title: required_str(object, "title", "department entry")?,
            enabled: required_bool(object, "enabled", "department entry")?,
            prompt: required_str(object, "prompt", "department entry")?,
            consumes,
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
        super::content_contract::reject_duplicate_json_keys(text)?;
        let value: Value = serde_json::from_str(text)
            .map_err(|error| format!("department registry is not JSON: {error}"))?;
        Self::from_value(&value)
    }

    pub(super) fn from_value(value: &Value) -> Result<Self, String> {
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
        self.get(id).is_some()
    }

    /// The registered department entry for `id`, if any. Used by
    /// `AuditReport::validate()` to check the registry's `enabled` flag
    /// against the report's department run status.
    pub(crate) fn get(&self, id: &str) -> Option<&DepartmentEntry> {
        self.departments
            .iter()
            .find(|department| department.id == id)
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
#[path = "registry_tests.rs"]
mod tests;
