use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::artifact_ref::VerifiedArtifact;
use crate::capability_inventory::{AdapterArtifact, AdapterError, AdapterOutput};

#[derive(Default)]
struct Signals {
    local_tool_failure: bool,
    provider_quota: bool,
    graph_seen: bool,
    graph_current: bool,
    structural_seen: bool,
    structural_trusted: bool,
    structural_rules: bool,
    structural_failure: bool,
    native_seen: bool,
    modernization_debt: bool,
    top_target: Option<String>,
    admissions: BTreeMap<String, String>,
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
    if !options.is_empty() {
        return Err(AdapterError::InvalidOptions(
            "diagnosis.hospital accepts no options".into(),
        ));
    }
    if verified_inputs.is_empty() {
        return Err(AdapterError::Contract(
            "diagnosis.hospital requires A04 admission Artifact Refs".into(),
        ));
    }
    let mut signals = Signals::default();
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
        Some("pass") => crate::capability_inventory::AdapterDomainVerdict::Pass,
        Some("fail") => crate::capability_inventory::AdapterDomainVerdict::Fail,
        Some("unknown") => crate::capability_inventory::AdapterDomainVerdict::Unknown,
        Some("not_applicable") => crate::capability_inventory::AdapterDomainVerdict::NotApplicable,
        other => {
            return Err(AdapterError::Contract(format!(
                "hospital diagnosis has unsupported domain verdict: {other:?}"
            )))
        }
    };
    let domain_failure =
        (domain_verdict == crate::capability_inventory::AdapterDomainVerdict::Fail).then(|| {
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
    } else if !s.structural_seen || !s.structural_trusted {
        (
            "unknown",
            "authoritative structural evidence unavailable",
            "diagnose",
            "admit",
            "unknown",
        )
    } else if !s.structural_rules {
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
            "overall_score":null,
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
        "report_quality":{"overall_score":null,"diagnostic_score":null,"governance_score":null,"dimensions":[]},
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
