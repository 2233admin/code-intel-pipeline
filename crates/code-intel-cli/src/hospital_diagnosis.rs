use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::adapter_contract::{AdapterArtifact, AdapterDomainVerdict, AdapterError, AdapterOutput};
use crate::artifact_ref::VerifiedArtifact;
use crate::audit_report::AuditReport;

struct Signals {
    local_tool_failure: bool,
    provider_quota: bool,
    graph_seen: bool,
    graph_current: bool,
    /// Whether the run was configured to produce structural evidence at all.
    ///
    /// Absence and exclusion are different facts. A run that was asked for the
    /// structural stage and got nothing has an evidence gap and must not be
    /// certified; a run that was never asked for it (`--mode lite`) simply has
    /// a narrower scope, and reporting that as "unavailable" would make every
    /// deliberately-narrow run indistinguishable from a broken one.
    ///
    /// Only the execution policy may set this false, and the policy already
    /// refuses to disable a capability its profile marks `Required` — so this
    /// cannot become a way to talk the hospital out of a gate that a strict
    /// run demanded. The resulting narrowing is recorded in the report.
    structural_in_scope: bool,
    structural_seen: bool,
    structural_trusted: bool,
    structural_rules: bool,
    structural_failure: bool,
    native_seen: bool,
    modernization_debt: bool,
    top_target: Option<String>,
    failing_rules: Vec<Value>,
    sentrux_capability_refs: Vec<Value>,
    admissions: BTreeMap<String, String>,
}

impl Default for Signals {
    fn default() -> Self {
        Self {
            local_tool_failure: false,
            provider_quota: false,
            graph_seen: false,
            graph_current: false,
            // Fail closed: unless a run explicitly declares the structural
            // stage out of scope, its absence stays an evidence gap.
            structural_in_scope: true,
            structural_seen: false,
            structural_trusted: false,
            structural_rules: false,
            structural_failure: false,
            native_seen: false,
            modernization_debt: false,
            top_target: None,
            failing_rules: Vec::new(),
            sentrux_capability_refs: Vec::new(),
            admissions: BTreeMap::new(),
        }
    }
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
    // `structuralEvidenceInScope` is the only option, and it may only ever
    // narrow: the caller declares that the run never asked for the structural
    // stage. Anything else is rejected so the option surface cannot grow into
    // a way to hand-tune a diagnosis.
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
    let machine = diagnose(request, &signals, None);
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
    let hospital_markdown = render_hospital(&machine, None).into_bytes();
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
    if let Some(refs) = data.get("capabilityArtifactRefs").and_then(Value::as_array) {
        require_provider_modality(provider, "structural_evidence")?;
        if !signals.sentrux_capability_refs.is_empty() {
            return Err(AdapterError::Contract(
                "duplicate admitted Sentrux capability artifacts".into(),
            ));
        }
        signals.sentrux_capability_refs = refs.clone();
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

fn diagnose(request: &Value, s: &Signals, audit: Option<&AuditReport>) -> Value {
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
    let structural_target = s.failing_rules.iter().find_map(|rule| {
        rule["details"]["violations"]
            .as_array()
            .and_then(|violations| {
                violations.iter().find_map(|violation| {
                    violation["targets"]
                        .as_array()
                        .and_then(|targets| targets.first())
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
            })
    });
    let surgery_target = if diagnosis == "architecture gate failure" {
        structural_target.clone().or_else(|| s.top_target.clone())
    } else {
        s.top_target.clone()
    };
    let treatment = treatment(diagnosis, surgery_target.as_deref(), &s.failing_rules);
    let surgery_status = if (next_protocol == "surgery_plan" && s.top_target.is_some())
        || (diagnosis == "architecture gate failure" && surgery_target.is_some())
    {
        "planned"
    } else {
        "not_required"
    };
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
    let evidence = s
        .admissions
        .iter()
        .map(|(provider, admission)| json!({"provider":provider,"admissionIdentity":admission}))
        .collect::<Vec<_>>();
    let mut machine = json!({
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
        "policies":{"precedence":["local tool failure","provider quota exhausted","architecture gate failure","architecture graph missing","authoritative structural evidence unavailable","ungoverned structural scope","known modernization debt","clean snapshot"],"scope":{"structuralEvidence":if s.structural_in_scope {"in_scope"} else {"out_of_scope"}}},
        "report_quality":{"overall_score":null,"diagnostic_score":null,"governance_score":null,"dimensions":[]},
        "diagnosis":{"findings":[diagnosis],"impression":diagnosis,"risk":status,"evidence":evidence},
        "treatment":{"plan":treatment,"follow_up":["Rerun diagnosis.hospital with current admitted evidence."]},
        "protocols":[],
        "tools":{"sentruxCapabilities":s.sentrux_capability_refs},
        "surgery_plan":{
            "schema":"code-intel-surgery-plan.v1",
            "status":surgery_status,
            "admission":{"disposition":disposition,"diagnosis":diagnosis,"reason":admission_reason(diagnosis)},
            "primary_target":{"file":surgery_target,"name":null,"source_anchor":null,"complexity":null,"scenario":null,"scenario_action":null,"codenexus_file":null},
            "operating_plan":if surgery_status == "planned" { vec!["Open the admitted primary target before editing.","Make one bounded repair and preserve behavior."] } else { Vec::<&str>::new() },
            "verification":["Rerun the smallest affected test.","Re-admit current structural evidence before discharge."],
            "discharge_criteria":["the admitted structural verdict is pass"]
        }
    });
    if let Some(report) = audit {
        machine["audit"] = report.summary("audit-report.json").to_value();
    }
    machine
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

fn treatment(diagnosis: &str, target: Option<&str>, failing: &[Value]) -> Vec<String> {
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
    if diagnosis == "architecture gate failure" {
        for rule in failing {
            let kind = rule["kind"].as_str().unwrap_or("unknown");
            let violations = rule["details"]["violations"].as_array();
            match violations {
                Some(violations) => {
                    for violation in violations.iter().take(3) {
                        let message = violation["message"].as_str().unwrap_or("");
                        let targets = violation["targets"]
                            .as_array()
                            .map(|targets| {
                                targets
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .take(3)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default();
                        if targets.is_empty() {
                            plan.push(format!("Failing rule {kind}: {message}."));
                        } else {
                            plan.push(format!(
                                "Failing rule {kind}: {message} (targets: {targets})."
                            ));
                        }
                    }
                }
                None => plan.push(format!(
                    "Failing rule {kind}: no structured violation details were admitted."
                )),
            }
        }
        plan.push(
            "Rerun the smallest gate: code-intel sentrux --operation check --repo <repo-root>."
                .into(),
        );
    }
    if let Some(target) = target {
        plan.push(format!("Start the bounded review at {target}."));
    }
    plan
}

fn render_hospital(value: &Value, audit: Option<&AuditReport>) -> String {
    let mut report = format!(
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
    );
    let failing = value["triage"]["failing_rules"].as_array();
    if let Some(failing) = failing.filter(|rules| !rules.is_empty()) {
        report.push_str("\n## Failing rules\n");
        for rule in failing {
            let kind = rule["kind"].as_str().unwrap_or("unknown");
            match rule["details"]["violations"].as_array() {
                Some(violations) => {
                    for violation in violations {
                        let message = violation["message"].as_str().unwrap_or("");
                        let targets = violation["targets"]
                            .as_array()
                            .map(|targets| {
                                targets
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default();
                        if targets.is_empty() {
                            report.push_str(&format!("- {kind}: {message}\n"));
                        } else {
                            report.push_str(&format!("- {kind}: {message} (targets: {targets})\n"));
                        }
                    }
                }
                None => report.push_str(&format!("- {kind}: no structured violation details\n")),
            }
        }
    }
    if let Some(report_data) = audit {
        report.push_str(&crate::audit_report::render_markdown_section(report_data));
    }
    report
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

#[cfg(test)]
mod audit_wiring_tests {
    use super::*;

    fn clean_signals() -> Signals {
        Signals {
            graph_seen: true,
            graph_current: true,
            structural_seen: true,
            structural_trusted: true,
            structural_rules: true,
            ..Signals::default()
        }
    }

    fn sample_request() -> Value {
        json!({"snapshot": {"repoIdentity": "content-v1:test"}})
    }

    fn sample_audit_report() -> AuditReport {
        let bytes = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/audit/audit-report.v1.example.json"),
        )
        .unwrap();
        AuditReport::parse(&bytes).unwrap()
    }

    #[test]
    fn audit_absent_omits_the_audit_key_and_section() {
        let machine = diagnose(&sample_request(), &clean_signals(), None);
        assert!(machine.get("audit").is_none());
        let markdown = render_hospital(&machine, None);
        assert!(!markdown.contains("## Audit"));
    }

    /// A run that never asked for the structural stage has a narrower scope,
    /// not an evidence gap. Before this distinction existed, `--mode lite`
    /// exited 20 on a clean repository because absence was read as a gap.
    #[test]
    fn structural_evidence_out_of_scope_reaches_a_verdict_instead_of_unknown() {
        let signals = Signals {
            graph_seen: true,
            graph_current: true,
            structural_in_scope: false,
            ..Signals::default()
        };
        let machine = diagnose(&sample_request(), &signals, None);
        assert_eq!(machine["domainVerdict"], "pass");
        assert_eq!(machine["diagnosis"]["impression"], "clean snapshot");
        // The narrowing has to be visible in the artifact, never silent.
        assert_eq!(
            machine["policies"]["scope"]["structuralEvidence"],
            "out_of_scope"
        );
    }

    /// The fail-closed half: in scope and absent is still a gap.
    #[test]
    fn structural_evidence_in_scope_but_absent_stays_unknown() {
        let signals = Signals {
            graph_seen: true,
            graph_current: true,
            ..Signals::default()
        };
        let machine = diagnose(&sample_request(), &signals, None);
        assert_eq!(machine["domainVerdict"], "unknown");
        assert_eq!(
            machine["diagnosis"]["impression"],
            "authoritative structural evidence unavailable"
        );
        assert_eq!(
            machine["policies"]["scope"]["structuralEvidence"],
            "in_scope"
        );
    }

    /// Defence in depth: admitted evidence beats a scope claim. If structural
    /// evidence was actually produced and it fails, declaring it out of scope
    /// must not launder the failure into a pass — otherwise the option would
    /// be a way to talk the hospital out of a gate it already has evidence for.
    #[test]
    fn an_out_of_scope_claim_cannot_launder_an_admitted_structural_failure() {
        let signals = Signals {
            graph_seen: true,
            graph_current: true,
            structural_in_scope: false,
            structural_seen: true,
            structural_trusted: true,
            structural_rules: true,
            structural_failure: true,
            ..Signals::default()
        };
        let machine = diagnose(&sample_request(), &signals, None);
        assert_eq!(machine["domainVerdict"], "fail");
        assert_eq!(
            machine["diagnosis"]["impression"],
            "architecture gate failure"
        );
    }

    /// Scope narrowing never outranks a real failure signal that precedes it
    /// in the ladder.
    #[test]
    fn out_of_scope_does_not_mask_local_tool_failure_or_quota_exhaustion() {
        for (label, signals) in [
            (
                "local tool failure",
                Signals {
                    local_tool_failure: true,
                    structural_in_scope: false,
                    ..Signals::default()
                },
            ),
            (
                "provider quota exhausted",
                Signals {
                    provider_quota: true,
                    structural_in_scope: false,
                    ..Signals::default()
                },
            ),
        ] {
            let machine = diagnose(&sample_request(), &signals, None);
            assert_eq!(machine["domainVerdict"], "unknown", "{label}");
            assert_eq!(machine["diagnosis"]["impression"], label);
        }
    }

    /// A missing architecture graph outranks the structural scope question,
    /// so lite runs still cannot certify without one.
    #[test]
    fn out_of_scope_does_not_excuse_a_missing_architecture_graph() {
        let signals = Signals {
            structural_in_scope: false,
            ..Signals::default()
        };
        let machine = diagnose(&sample_request(), &signals, None);
        assert_eq!(machine["domainVerdict"], "unknown");
        assert_eq!(
            machine["diagnosis"]["impression"],
            "architecture graph missing"
        );
    }

    #[test]
    fn audit_present_embeds_the_summary_and_renders_the_section() {
        let report = sample_audit_report();
        let machine = diagnose(&sample_request(), &clean_signals(), Some(&report));
        assert_eq!(machine["audit"]["status"], "present");
        assert_eq!(machine["audit"]["artifact"], "audit-report.json");
        assert_eq!(machine["audit"]["overall"], 7.0);
        assert_eq!(machine["audit"]["findings_total"], 1);
        assert_eq!(machine["audit"]["by_severity"]["medium"], 1);

        let markdown = render_hospital(&machine, Some(&report));
        assert!(markdown.contains("## Audit"));
        assert!(markdown.contains("medium | security-001 |"));
    }
}
