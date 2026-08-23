use std::collections::BTreeSet;
use std::path::{Component, Path};

use serde_json::{json, Value};

/// Evaluates agent-facing diagnosis invariants without changing the v1 artifact.
///
/// The result is intentionally private until the dimensions receive a versioned
/// schema item contract and artifact-level compatibility tests.
pub(super) fn report_quality_dimensions(machine: &Value, request: &Value) -> Vec<Value> {
    let (anchored, anchor_evidence) = assess_evidence_anchors(machine);
    let (verifiable, plan_evidence) = assess_plan_verification(machine);
    let (in_scope, scope_evidence) = assess_scope(machine, request);
    vec![
        quality_dimension("missing_evidence_anchor", anchored, anchor_evidence),
        quality_dimension("plan_without_verification", verifiable, plan_evidence),
        quality_dimension("scope_leak", in_scope, scope_evidence),
    ]
}

fn quality_dimension(id: &str, passed: bool, evidence: Vec<String>) -> Value {
    json!({
        "id": id,
        "status": if passed { "pass" } else { "fail" },
        "evidence": evidence
    })
}

fn assess_evidence_anchors(machine: &Value) -> (bool, Vec<String>) {
    let modalities = machine["modalities"].as_array();
    let claims = machine
        .pointer("/diagnosis/evidence")
        .and_then(Value::as_array);
    let mut evidence = Vec::new();

    let modalities_valid = modalities
        .is_some_and(|items| !items.is_empty() && items.iter().all(valid_admission_descriptor));
    if !modalities_valid {
        evidence.push("diagnosis has no valid admitted modality anchor".into());
    }

    let claims_valid = claims.is_some_and(|items| {
        !items.is_empty()
            && items.iter().all(valid_admission_descriptor)
            && modalities.is_some_and(|admitted| {
                items.iter().all(|claim| {
                    admitted.iter().any(|modality| {
                        modality["provider"] == claim["provider"]
                            && modality["admissionIdentity"] == claim["admissionIdentity"]
                    })
                })
            })
    });
    if !claims_valid {
        evidence.push("diagnosis evidence does not resolve to an admitted modality".into());
    }

    if let Some(rules) = machine
        .pointer("/triage/failing_rules")
        .and_then(Value::as_array)
    {
        for (index, rule) in rules.iter().enumerate() {
            let structured = rule
                .pointer("/details/violations")
                .and_then(Value::as_array)
                .is_some_and(|violations| {
                    !violations.is_empty()
                        && violations.iter().all(|violation| {
                            violation["rule"]
                                .as_str()
                                .is_some_and(|value| !value.is_empty())
                                && violation["message"]
                                    .as_str()
                                    .is_some_and(|value| !value.is_empty())
                                && violation["targets"].as_array().is_some_and(|targets| {
                                    !targets.is_empty()
                                        && targets.iter().all(|target| {
                                            target
                                                .as_str()
                                                .is_some_and(|value| !value.trim().is_empty())
                                        })
                                })
                        })
                });
            if !structured {
                evidence.push(format!(
                    "failing rule at index {index} has no structured evidence"
                ));
            }
        }
    }

    (evidence.is_empty(), evidence)
}

fn valid_admission_descriptor(value: &Value) -> bool {
    value["provider"]
        .as_str()
        .is_some_and(|provider| !provider.is_empty())
        && value["admissionIdentity"]
            .as_str()
            .is_some_and(is_lower_hex_digest)
}

fn is_lower_hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn assess_plan_verification(machine: &Value) -> (bool, Vec<String>) {
    let plan = &machine["surgery_plan"];
    if plan["status"] != "planned" {
        return (true, Vec::new());
    }

    let mut evidence = Vec::new();
    if !plan
        .pointer("/primary_target/file")
        .and_then(Value::as_str)
        .is_some_and(|file| !file.is_empty())
    {
        evidence.push("planned surgery has no primary target".into());
    }
    for (field, label) in [
        ("operating_plan", "operating action"),
        ("verification", "verification"),
        ("discharge_criteria", "discharge criterion"),
    ] {
        let valid = plan[field].as_array().is_some_and(|items| {
            !items.is_empty()
                && items
                    .iter()
                    .all(|item| item.as_str().is_some_and(|value| !value.trim().is_empty()))
        });
        if !valid {
            evidence.push(format!("planned surgery has no {label}"));
        }
    }
    (evidence.is_empty(), evidence)
}

fn assess_scope(machine: &Value, request: &Value) -> (bool, Vec<String>) {
    let targets = target_paths(machine);
    if targets.is_empty() {
        return (true, Vec::new());
    }
    let scopes = request
        .pointer("/snapshot/scope")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter_map(normalize_repo_path)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if scopes.is_empty() {
        return (
            false,
            vec!["snapshot scope is missing for planned targets".into()],
        );
    }

    let mut evidence = Vec::new();
    for target in targets {
        let Some(normalized_target) = normalize_repo_path(&target) else {
            evidence.push(format!("target is not repository-relative: {target}"));
            continue;
        };
        if !scopes.iter().any(|scope| {
            scope == "."
                || normalized_target == *scope
                || normalized_target.starts_with(&format!("{scope}/"))
        }) {
            evidence.push(format!("target is outside snapshot scope: {target}"));
        }
    }
    (evidence.is_empty(), evidence)
}

fn target_paths(machine: &Value) -> Vec<String> {
    let mut targets = BTreeSet::new();
    if let Some(target) = machine
        .pointer("/surgery_plan/primary_target/file")
        .and_then(Value::as_str)
    {
        targets.insert(target.to_string());
    }
    if let Some(rules) = machine
        .pointer("/triage/failing_rules")
        .and_then(Value::as_array)
    {
        for target in rules
            .iter()
            .filter_map(|rule| rule.pointer("/details/violations"))
            .filter_map(Value::as_array)
            .flat_map(|violations| violations.iter())
            .filter_map(|violation| violation["targets"].as_array())
            .flat_map(|items| items.iter())
            .filter_map(Value::as_str)
        {
            targets.insert(target.to_string());
        }
    }
    targets.into_iter().collect()
}

fn normalize_repo_path(value: &str) -> Option<String> {
    if value.is_empty()
        || value.contains('\\')
        || value.starts_with('/')
        || value.starts_with('~')
        || value.as_bytes().get(1) == Some(&b':')
    {
        return None;
    }
    if Path::new(value).components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return None;
    }
    let normalized = value.trim_start_matches("./").trim_matches('/').to_string();
    Some(if normalized.is_empty() {
        ".".into()
    } else {
        normalized
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quality_dimension<'a>(dimensions: &'a [Value], id: &str) -> &'a Value {
        dimensions
            .iter()
            .find(|dimension| dimension["id"] == id)
            .unwrap_or_else(|| panic!("missing quality dimension {id}"))
    }

    fn quality_request(scope: Value) -> Value {
        json!({
            "snapshot": {
                "repoIdentity": "content-v1:test",
                "scope": scope
            }
        })
    }

    fn quality_machine() -> Value {
        let admission_identity = "a".repeat(64);
        json!({
            "triage": {
                "primary_diagnosis": "clean snapshot",
                "failing_rules": []
            },
            "modalities": [{
                "provider": "architecture-graph.internal",
                "admissionIdentity": admission_identity
            }],
            "diagnosis": {
                "evidence": [{
                    "provider": "architecture-graph.internal",
                    "admissionIdentity": admission_identity
                }]
            },
            "surgery_plan": {
                "status": "not_required",
                "primary_target": {"file": null},
                "operating_plan": [],
                "verification": ["Rerun the smallest affected test."],
                "discharge_criteria": ["the admitted structural verdict is pass"]
            }
        })
    }

    #[test]
    fn report_quality_dimensions_pass_for_anchored_clean_report() {
        let dimensions =
            report_quality_dimensions(&quality_machine(), &quality_request(json!(["."])));
        let replay = report_quality_dimensions(&quality_machine(), &quality_request(json!(["."])));
        assert_eq!(dimensions, replay);
        assert_eq!(
            dimensions
                .iter()
                .filter_map(|dimension| dimension["id"].as_str())
                .collect::<Vec<_>>(),
            vec![
                "missing_evidence_anchor",
                "plan_without_verification",
                "scope_leak"
            ]
        );
        assert_eq!(
            quality_dimension(&dimensions, "missing_evidence_anchor")["status"],
            "pass"
        );
        assert_eq!(
            quality_dimension(&dimensions, "plan_without_verification")["status"],
            "pass"
        );
        assert_eq!(
            quality_dimension(&dimensions, "scope_leak")["status"],
            "pass"
        );
    }

    #[test]
    fn report_quality_dimensions_reject_unanchored_failing_rules() {
        let mut machine = quality_machine();
        machine["triage"]["failing_rules"] = json!([{
            "kind": "max_cycles",
            "details": {
                "violations": [{
                    "rule": "max_cycles",
                    "message": "cycles exceeded",
                    "targets": []
                }]
            }
        }]);

        let dimensions = report_quality_dimensions(&machine, &quality_request(json!(["."])));

        assert_eq!(
            quality_dimension(&dimensions, "missing_evidence_anchor")["status"],
            "fail"
        );
    }

    #[test]
    fn report_quality_dimensions_reject_null_rule_targets() {
        let mut machine = quality_machine();
        machine["triage"]["failing_rules"] = json!([{
            "kind": "max_cycles",
            "details": {
                "violations": [{
                    "rule": "max_cycles",
                    "message": "cycles exceeded",
                    "targets": [null]
                }]
            }
        }]);

        let dimensions = report_quality_dimensions(&machine, &quality_request(json!(["."])));
        assert_eq!(
            quality_dimension(&dimensions, "missing_evidence_anchor")["status"],
            "fail"
        );
    }

    #[test]
    fn normalize_repo_path_treats_root_alias_as_repository_scope() {
        assert_eq!(normalize_repo_path("./"), Some(".".into()));
        assert_eq!(normalize_repo_path("./src"), Some("src".into()));
        assert_eq!(normalize_repo_path("../src"), None);
    }

    #[test]
    fn report_quality_dimensions_identify_missing_evidence_anchor() {
        let mut machine = quality_machine();
        machine["diagnosis"]["evidence"] = json!([]);

        let dimensions = report_quality_dimensions(&machine, &quality_request(json!(["."])));

        assert_eq!(
            quality_dimension(&dimensions, "missing_evidence_anchor")["status"],
            "fail"
        );
    }

    #[test]
    fn report_quality_dimensions_identify_plan_without_verification() {
        let mut machine = quality_machine();
        machine["surgery_plan"]["status"] = json!("planned");
        machine["surgery_plan"]["primary_target"]["file"] = json!("src/lib.rs");
        machine["surgery_plan"]["operating_plan"] = json!(["Make one bounded repair."]);
        machine["surgery_plan"]["verification"] = json!([]);

        let dimensions = report_quality_dimensions(&machine, &quality_request(json!(["src"])));

        assert_eq!(
            quality_dimension(&dimensions, "plan_without_verification")["status"],
            "fail"
        );
    }

    #[test]
    fn report_quality_dimensions_identify_scope_leak() {
        let mut machine = quality_machine();
        machine["surgery_plan"]["status"] = json!("planned");
        machine["surgery_plan"]["primary_target"]["file"] = json!("tests/lib.rs");
        machine["surgery_plan"]["operating_plan"] = json!(["Make one bounded repair."]);

        let dimensions = report_quality_dimensions(&machine, &quality_request(json!(["src"])));

        assert_eq!(
            quality_dimension(&dimensions, "scope_leak")["status"],
            "fail"
        );
    }

    #[test]
    fn report_quality_dimensions_keep_legal_identifiers_commands_and_paths() {
        let mut machine = quality_machine();
        machine["surgery_plan"]["status"] = json!("planned");
        machine["surgery_plan"]["primary_target"]["file"] =
            json!("crates/code-intel-cli/src/hospital_diagnosis.rs");
        machine["surgery_plan"]["operating_plan"] = json!([
            "Open hospital_diagnosis.rs before editing.",
            "Keep diagnosis.hospital.compat as the runtime adapter."
        ]);
        machine["surgery_plan"]["verification"] = json!([
            "cargo test -p code-intel --test hospital_diagnosis",
            "cargo test -p code-intel report_quality_dimensions",
            "code-intel lint hardcoded-paths ."
        ]);
        machine["triage"]["failing_rules"] = json!([{
            "kind": "max_cycles",
            "details": {
                "violations": [{
                    "rule": "max_cycles",
                    "message": "cycles exceeded",
                    "targets": [
                        "crates/code-intel-cli/src/hospital_diagnosis.rs",
                        "crates/code-intel-cli/src/report_quality.rs"
                    ]
                }]
            }
        }]);

        let request = quality_request(json!(["crates/code-intel-cli"]));
        let dimensions = report_quality_dimensions(&machine, &request);
        let replay = report_quality_dimensions(&machine, &request);
        assert_eq!(dimensions, replay);
        assert_eq!(
            quality_dimension(&dimensions, "missing_evidence_anchor")["status"],
            "pass"
        );
        assert_eq!(
            quality_dimension(&dimensions, "plan_without_verification")["status"],
            "pass"
        );
        assert_eq!(
            quality_dimension(&dimensions, "scope_leak")["status"],
            "pass"
        );
    }

    #[test]
    fn report_quality_dimensions_fail_closed_without_admitted_modalities() {
        let mut machine = quality_machine();
        machine["modalities"] = json!([]);

        let dimensions = report_quality_dimensions(&machine, &quality_request(json!(["."])));
        let replay = report_quality_dimensions(&machine, &quality_request(json!(["."])));
        assert_eq!(dimensions, replay);
        assert_eq!(
            quality_dimension(&dimensions, "missing_evidence_anchor")["status"],
            "fail"
        );
    }

    #[test]
    fn report_quality_dimensions_fail_closed_when_scope_missing() {
        let mut machine = quality_machine();
        machine["surgery_plan"]["status"] = json!("planned");
        machine["surgery_plan"]["primary_target"]["file"] = json!("src/lib.rs");

        let dimensions = report_quality_dimensions(&machine, &quality_request(json!([])));
        assert_eq!(
            quality_dimension(&dimensions, "scope_leak")["status"],
            "fail"
        );
    }
}
