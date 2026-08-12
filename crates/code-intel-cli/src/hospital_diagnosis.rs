use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::{
    adapter_contract::{AdapterArtifact, AdapterDomainVerdict, AdapterError, AdapterOutput},
    artifact_ref::VerifiedArtifact,
};

#[derive(Default)]
struct Signals {
    local_tool_failure: bool,
    provider_quota: bool,
    graph_seen: bool,
    graph_current: bool,
    structural_in_scope: bool,
    structural_seen: bool,
    structural_trusted: bool,
    structural_rules: bool,
    structural_failure: bool,
    native_seen: bool,
    modernization_debt: bool,
    top_target: Option<String>,
    failing_rules: Vec<Value>,
    admissions: BTreeMap<String, String>,
}

struct ReadinessDimension {
    id: &'static str,
    name: &'static str,
    normalized: f64,
    evidence: String,
}

pub(crate) fn execute(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    let options = request
        .get("options")
        .and_then(Value::as_object)
        .ok_or_else(|| AdapterError::InvalidOptions("options must be an object".into()))?;
    let mut structural_in_scope = true;
    for (key, value) in options {
        match (key.as_str(), value.as_bool()) {
            ("structuralEvidenceInScope", Some(in_scope)) => structural_in_scope = in_scope,
            ("structuralEvidenceInScope", None) => {
                return Err(AdapterError::InvalidOptions(
                    "diagnosis.hospital structuralEvidenceInScope must be boolean".into(),
                ))
            }
            _ => {
                return Err(AdapterError::InvalidOptions(
                    "diagnosis.hospital accepts only structuralEvidenceInScope".into(),
                ))
            }
        }
    }
    if verified_inputs.is_empty() {
        return Err(AdapterError::Contract(
            "diagnosis.hospital requires A04 admission Artifact Refs".into(),
        ));
    }
    let mut signals = Signals {
        structural_in_scope,
        ..Signals::default()
    };
    for input in verified_inputs {
        if input.artifact_schema() != "code-intel-evidence-admissibility-result.v1"
            || input.artifact_type() != "evidence.admission"
        {
            return Err(AdapterError::Contract(
                "diagnosis.hospital consumes only A04 admission Artifact Refs".into(),
            ));
        }
        consume_admission(input, &mut signals)?;
    }
    let machine = diagnose(request, &signals);
    let domain_verdict = match machine["domainVerdict"].as_str() {
        Some("pass") => AdapterDomainVerdict::Pass,
        Some("fail") => AdapterDomainVerdict::Fail,
        Some("unknown") => AdapterDomainVerdict::Unknown,
        Some("not_applicable") => AdapterDomainVerdict::NotApplicable,
        other => {
            return Err(AdapterError::Contract(format!(
                "hospital diagnosis has unsupported domain verdict: {other:?}"
            )))
        }
    };
    let domain_failure = (domain_verdict == AdapterDomainVerdict::Fail).then(|| {
        machine["triage"]["primary_diagnosis"]
            .as_str()
            .unwrap_or("hospital domain failure")
            .to_string()
    });
    let surgery = machine["surgery_plan"].clone();
    let hospital_bytes = serde_json::to_vec(&machine)
        .map_err(|error| AdapterError::Internal(format!("serialize hospital report: {error}")))?;
    let surgery_bytes = serde_json::to_vec(&surgery)
        .map_err(|error| AdapterError::Internal(format!("serialize surgery plan: {error}")))?;
    let hospital_markdown = render_hospital(&machine).into_bytes();
    let surgery_markdown = render_surgery(&surgery).into_bytes();
    fs::create_dir_all(out)
        .map_err(|error| AdapterError::Io(format!("create hospital output directory: {error}")))?;
    for (name, bytes) in [
        ("hospital-report.json", hospital_bytes.as_slice()),
        ("hospital.md", hospital_markdown.as_slice()),
        ("surgery-plan.json", surgery_bytes.as_slice()),
        ("surgery-plan.md", surgery_markdown.as_slice()),
    ] {
        fs::write(out.join(name), bytes)
            .map_err(|error| AdapterError::Io(format!("write {name}: {error}")))?;
    }
    Ok(AdapterOutput {
        artifacts: vec![
            artifact(
                "code-intel-hospital.v1",
                "diagnosis.hospital",
                "hospital-report.json",
                hospital_bytes,
            ),
            artifact(
                "code-intel-hospital-markdown.v1",
                "diagnosis.hospital-view",
                "hospital.md",
                hospital_markdown,
            ),
            artifact(
                "code-intel-surgery-plan.v1",
                "diagnosis.surgery-plan",
                "surgery-plan.json",
                surgery_bytes,
            ),
            artifact(
                "code-intel-surgery-plan-markdown.v1",
                "diagnosis.surgery-plan-view",
                "surgery-plan.md",
                surgery_markdown,
            ),
        ],
        observed_effects: vec!["local_write".into()],
        domain_verdict,
        domain_failure,
    })
}

fn artifact(schema: &str, kind: &str, path: &str, bytes: Vec<u8>) -> AdapterArtifact {
    AdapterArtifact {
        artifact_schema: schema.into(),
        artifact_type: kind.into(),
        relative_path: path.into(),
        bytes,
    }
}

fn consume_admission(input: &VerifiedArtifact, signals: &mut Signals) -> Result<(), AdapterError> {
    let admission: Value = serde_json::from_slice(input.bytes())
        .map_err(|error| AdapterError::Contract(format!("parse A04 admission: {error}")))?;
    if admission["status"] != "admitted" {
        return Err(AdapterError::Contract(
            "non-admitted evidence cannot enter diagnosis.hospital".into(),
        ));
    }
    let provider = admission["evidence"]["provider"]["id"]
        .as_str()
        .ok_or_else(|| AdapterError::Contract("A04 admission lacks provider id".into()))?;
    let identity = admission["admissionIdentity"]
        .as_str()
        .ok_or_else(|| AdapterError::Contract("A04 admission lacks identity".into()))?;
    if signals
        .admissions
        .insert(provider.to_string(), identity.to_string())
        .is_some()
    {
        return Err(AdapterError::Contract(format!(
            "duplicate admitted modality: {provider}"
        )));
    }
    let verdict = admission["domainVerdict"].as_str().unwrap_or("unknown");
    let failure = admission["evidence"]["failure"]["kind"]
        .as_str()
        .unwrap_or("domain_unknown");
    signals.local_tool_failure |= failure == "local_tool_error";
    let data = &admission["verifiedPayload"]["data"];
    if data.get("repowise").is_some() {
        require_provider_modality(provider, "repowise")?;
    }
    signals.provider_quota |= matches!(provider, "repowise.docs" | "repowise.index")
        && (failure == "provider_unavailable" || data["repowise"]["status"] == "quota");
    if let Some(graph) = data.get("architectureGraph") {
        require_provider_modality(provider, "architecture_graph")?;
        if signals.graph_seen {
            return Err(AdapterError::Contract(
                "duplicate admitted authoritative modality: architecture_graph".into(),
            ));
        }
        signals.graph_seen = true;
        signals.graph_current = verdict == "observed"
            && graph["completeness"] == "complete"
            && graph["graph"].is_object();
    }
    if let Some(structural) = data.get("structuralEvidence") {
        require_provider_modality(provider, "structural_evidence")?;
        if signals.structural_seen {
            return Err(AdapterError::Contract(
                "duplicate admitted authoritative modality: structural_evidence".into(),
            ));
        }
        signals.structural_seen = true;
        let rules = structural["rules"].as_array();
        signals.structural_rules = rules.is_some_and(|items| !items.is_empty());
        signals.structural_trusted = verdict == "observed"
            && structural["completeness"] == "complete"
            && rules.is_some_and(|items| {
                items
                    .iter()
                    .all(|rule| matches!(rule["verdict"].as_str(), Some("pass" | "fail")))
            });
        signals.structural_failure =
            rules.is_some_and(|items| items.iter().any(|rule| rule["verdict"] == "fail"));
        if let Some(items) = rules {
            signals.failing_rules.extend(
                items
                    .iter()
                    .filter(|rule| rule["verdict"] == "fail")
                    .cloned(),
            );
        }
    }
    if let Some(native) = data.get("nativeCode") {
        require_provider_modality(provider, "native_code")?;
        if signals.native_seen {
            return Err(AdapterError::Contract(
                "duplicate admitted enrichment modality: native_code".into(),
            ));
        }
        signals.native_seen = true;
        signals.modernization_debt |= native["modernizationDebt"] == true;
        signals.top_target = native["topTarget"]
            .as_str()
            .filter(|target| !target.is_empty())
            .map(str::to_string);
    }
    Ok(())
}

fn require_provider_modality(provider: &str, modality: &str) -> Result<(), AdapterError> {
    let matches = match modality {
        "repowise" => matches!(provider, "repowise.docs" | "repowise.index"),
        "architecture_graph" => provider == "architecture-graph.internal",
        "structural_evidence" => provider == "structural-evidence.sentrux",
        "native_code" => provider == "native-code-evidence",
        _ => false,
    };
    if matches {
        Ok(())
    } else {
        Err(AdapterError::Contract(format!(
            "provider identity cannot supply admitted modality {modality}"
        )))
    }
}

fn diagnose(request: &Value, s: &Signals) -> Value {
    let (status, diagnosis, next_protocol, disposition, domain_verdict) = if s.local_tool_failure {
        (
            "unknown",
            "local tool failure",
            "triage",
            "admit",
            "unknown",
        )
    } else if s.provider_quota {
        (
            "unknown",
            "provider quota exhausted",
            "triage",
            "admit",
            "unknown",
        )
    } else if s.structural_seen && s.structural_trusted && s.structural_failure {
        (
            "red",
            "architecture gate failure",
            "govern",
            "admit",
            "fail",
        )
    } else if !s.graph_seen || !s.graph_current {
        (
            "unknown",
            "architecture graph missing",
            "diagnose",
            "admit",
            "unknown",
        )
    } else if s.structural_in_scope && (!s.structural_seen || !s.structural_trusted) {
        (
            "unknown",
            "authoritative structural evidence unavailable",
            "diagnose",
            "admit",
            "unknown",
        )
    } else if s.structural_in_scope && !s.structural_rules {
        (
            "amber",
            "ungoverned structural scope",
            "govern",
            "admit",
            "fail",
        )
    } else if s.modernization_debt {
        (
            "amber",
            "known modernization debt",
            "surgery_plan",
            "admit",
            "fail",
        )
    } else {
        ("green", "clean snapshot", "post_op", "observe", "pass")
    };
    let treatment = treatment(diagnosis, s.top_target.as_deref());
    let surgery_status = if next_protocol == "surgery_plan" && s.top_target.is_some() {
        "planned"
    } else {
        "not_required"
    };
    let evidence = s
        .admissions
        .iter()
        .map(|(provider, admission)| json!({"provider":provider,"admissionIdentity":admission}))
        .collect::<Vec<_>>();
    let failing_rules = s
        .failing_rules
        .iter()
        .map(|rule| {
            json!({
                "kind": rule["kind"],
                "details": rule.get("details").cloned().unwrap_or(Value::Null),
            })
        })
        .collect::<Vec<_>>();
    let report_quality = development_readiness_signal(diagnosis, next_protocol, s);
    let overall_score = report_quality["overall_score"].clone();
    json!({
        "schema":"code-intel-hospital.v1",
        "domainVerdict":domain_verdict,
        "generatedAt":null,
        "repo":request["snapshot"]["repoIdentity"],
        "mode":"atom",
        "artifacts":{"runDir":"","report":"hospital-report.json","summary":"","understanding":""},
        "triage":{
            "status":status,
            "disposition":disposition,
            "primary_diagnosis":diagnosis,
            "failing_rules":failing_rules,
            "overall_score":overall_score,
            "next_protocol":next_protocol,
            "research_status":"not_applicable",
            "research_required":false,
            "exit_criteria":[],
            "admission_reason":admission_reason(diagnosis),
            "discharge_criteria":[
                "all required authoritative modalities are admitted and current",
                "structural rules contain no failing verdict",
                "post-op verification reports no regression"
            ]
        },
        "state_machine":{"schema":"code-intel-hospital-state-machine.v1","current_state":next_protocol,"disposition":disposition,"next_protocol":next_protocol,"states":["triage","diagnose","govern","surgery_plan","post_op","discharge_ready"],"transitions":[]},
        "modalities":evidence,
        "policies":{"precedence":["local tool failure","provider quota exhausted","architecture gate failure","architecture graph missing","authoritative structural evidence unavailable","ungoverned structural scope","known modernization debt","clean snapshot"]},
        "report_quality":report_quality,
        "diagnosis":{"findings":[diagnosis],"impression":diagnosis,"risk":status,"evidence":evidence},
        "treatment":{"plan":treatment,"follow_up":["Rerun diagnosis.hospital with current admitted evidence."]},
        "protocols":[],
        "tools":{},
        "surgery_plan":{
            "schema":"code-intel-surgery-plan.v1",
            "status":surgery_status,
            "admission":{"disposition":disposition,"diagnosis":diagnosis,"reason":admission_reason(diagnosis)},
            "primary_target":{"file":s.top_target,"name":null,"source_anchor":null,"complexity":null,"scenario":null,"scenario_action":null,"codenexus_file":null},
            "operating_plan":if surgery_status == "planned" { vec!["Open the admitted primary target before editing.","Make one bounded repair and preserve behavior."] } else { Vec::<&str>::new() },
            "verification":["Rerun the smallest affected test.","Re-admit current structural evidence before discharge."],
            "discharge_criteria":["the admitted structural verdict is pass"]
        }
    })
}

fn development_readiness_signal(diagnosis: &str, next_protocol: &str, s: &Signals) -> Value {
    let authoritative_seen = usize::from(s.graph_seen) + usize::from(s.structural_seen);
    let authoritative_current = usize::from(s.graph_current) + usize::from(s.structural_trusted);
    let coverage = authoritative_seen as f64 / 2.0;
    let currentness = if authoritative_seen == 0 {
        0.0
    } else {
        authoritative_current as f64 / authoritative_seen as f64
    };
    let governance = if s.structural_trusted && s.structural_rules {
        1.0
    } else {
        0.0
    };
    let availability = if !s.local_tool_failure && !s.provider_quota {
        1.0
    } else {
        0.0
    };
    let actionability = match diagnosis {
        "local tool failure" | "provider quota exhausted" => 0.0,
        "architecture graph missing" | "authoritative structural evidence unavailable" => 0.5,
        "known modernization debt" if s.top_target.is_none() => 0.5,
        _ => 1.0,
    };
    let mut dimensions = vec![
        ReadinessDimension {
            id: "authoritative_coverage",
            name: "Authoritative coverage",
            normalized: coverage,
            evidence: format!("required authoritative modalities={authoritative_seen}/2"),
        },
        ReadinessDimension {
            id: "evidence_currentness",
            name: "Evidence currentness",
            normalized: currentness,
            evidence: format!(
                "current admitted authoritative modalities={authoritative_current}/{authoritative_seen}"
            ),
        },
        ReadinessDimension {
            id: "governance_coverage",
            name: "Governance coverage",
            normalized: governance,
            evidence: format!(
                "trusted structural rules={}",
                if s.structural_trusted && s.structural_rules {
                    "present"
                } else {
                    "missing"
                }
            ),
        },
        ReadinessDimension {
            id: "tool_availability",
            name: "Tool availability",
            normalized: availability,
            evidence: format!(
                "local_tool_failure={}; provider_quota={}",
                s.local_tool_failure, s.provider_quota
            ),
        },
        ReadinessDimension {
            id: "actionability",
            name: "Actionability",
            normalized: actionability,
            evidence: format!(
                "diagnosis={diagnosis}; next_protocol={next_protocol}; bounded_target={}",
                s.top_target.as_deref().unwrap_or("none")
            ),
        },
    ];
    let weakest = dimensions
        .iter()
        .map(|dimension| dimension.normalized)
        .fold(1.0_f64, f64::min);
    let weakest_dimensions = if weakest >= 1.0 {
        Vec::new()
    } else {
        dimensions
            .iter()
            .filter(|dimension| (dimension.normalized - weakest).abs() < f64::EPSILON)
            .map(|dimension| dimension.id)
            .collect::<Vec<_>>()
    };
    let diagnostic_score = geometric_score(
        dimensions
            .iter()
            .filter(|dimension| dimension.id != "governance_coverage")
            .map(|dimension| dimension.normalized),
    );
    let governance_score = normalized_score(governance);
    let overall_score = geometric_score(dimensions.iter().map(|dimension| dimension.normalized));
    let dimension_values = dimensions
        .drain(..)
        .map(|dimension| {
            let is_weakest = weakest_dimensions.contains(&dimension.id);
            json!({
                "id":dimension.id,
                "name":dimension.name,
                "normalized":round_six(dimension.normalized),
                "score":normalized_score(dimension.normalized),
                "status":if dimension.normalized >= 1.0 { "ready" } else if dimension.normalized <= 0.0 { "blocked" } else { "partial" },
                "weakest":is_weakest,
                "evidence":dimension.evidence
            })
        })
        .collect::<Vec<_>>();
    json!({
        "overall_score":overall_score,
        "diagnostic_score":diagnostic_score,
        "governance_score":governance_score,
        "dimensions":dimension_values,
        "decision_signal":{
            "schema":"code-intel-development-readiness-signal.v1",
            "status":"experimental",
            "authority":"advisory_only",
            "meaning":"readiness to make the next development decision; not code health",
            "aggregation":"unweighted_geometric_mean",
            "scale":{"minimum":0,"maximum":100},
            "score":overall_score,
            "weakest_dimensions":weakest_dimensions
        }
    })
}

fn geometric_score(values: impl Iterator<Item = f64>) -> i64 {
    let values = values.collect::<Vec<_>>();
    if values.is_empty() || values.iter().any(|value| *value <= 0.0) {
        return 0;
    }
    normalized_score(
        (values.iter().map(|value| value.ln()).sum::<f64>() / values.len() as f64).exp(),
    )
}

fn normalized_score(value: f64) -> i64 {
    (value.clamp(0.0, 1.0) * 100.0).round() as i64
}

fn round_six(value: f64) -> f64 {
    (value.clamp(0.0, 1.0) * 1_000_000.0).round() / 1_000_000.0
}

fn admission_reason(diagnosis: &str) -> &'static str {
    match diagnosis {
        "local tool failure" => "Local execution failed before diagnosis could be trusted.",
        "provider quota exhausted" => "Provider quota prevented complete evidence collection.",
        "architecture gate failure" => "Admitted authoritative structural rules contain a failure.",
        "architecture graph missing" => "A current admitted architecture graph is unavailable.",
        "authoritative structural evidence unavailable" => {
            "Required authoritative structural evidence is missing or unknown."
        }
        "ungoverned structural scope" => {
            "No authoritative structural rules govern the selected scope."
        }
        "known modernization debt" => "Admitted evidence identifies bounded modernization debt.",
        _ => "No active inpatient diagnosis is present.",
    }
}

fn treatment(diagnosis: &str, target: Option<&str>) -> Vec<String> {
    let mut plan = vec![match diagnosis {
        "local tool failure" => "Fix local tool errors before interpreting architecture signals.".into(),
        "provider quota exhausted" => "Restore provider quota or use a complete admitted local evidence path before interpreting the result.".into(),
        "architecture gate failure" => "Repair the first failing admitted structural rule without weakening its threshold.".into(),
        "architecture graph missing" => "Produce and admit a current-snapshot architecture graph.".into(),
        "authoritative structural evidence unavailable" => "Produce and admit complete authoritative structural evidence.".into(),
        "ungoverned structural scope" => "Add and admit structural rules for the selected scope.".into(),
        "known modernization debt" => "Repair the first admitted modernization target and verify behavior.".into(),
        _ => "Keep this admitted evidence set as the clean comparison baseline.".into(),
    }];
    if let Some(target) = target {
        plan.push(format!("Start the bounded review at {target}."));
    }
    plan
}

fn render_hospital(value: &Value) -> String {
    format!(
        "# Code Intel Hospital Report\n\n- Status: {}\n- Disposition: {}\n- Primary diagnosis: {}\n- Next protocol: {}\n\n## Treatment\n{}\n",
        value["triage"]["status"].as_str().unwrap_or("unknown"),
        value["triage"]["disposition"].as_str().unwrap_or("admit"),
        value["triage"]["primary_diagnosis"].as_str().unwrap_or("unknown"),
        value["triage"]["next_protocol"].as_str().unwrap_or("triage"),
        value["treatment"]["plan"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

fn render_surgery(value: &Value) -> String {
    format!(
        "# Code Intel Surgery Plan\n\n- Status: {}\n- Diagnosis: {}\n",
        value["status"].as_str().unwrap_or("not_required"),
        value["admission"]["diagnosis"]
            .as_str()
            .unwrap_or("unknown")
    )
}
