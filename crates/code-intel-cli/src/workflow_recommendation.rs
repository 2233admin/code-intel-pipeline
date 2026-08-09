//! Deterministic, proposal-only workflow adapter recommendation.
//!
//! Candidate facts are loaded from the pipeline-owned catalog. This module
//! never invokes, initializes, installs, or adopts a candidate runtime.

// This adapter is owned by `capability_inventory`; inherit its already loaded
// contract and standard-library types instead of creating duplicate module
// edges for the same capability boundary.
use super::*;
use serde_json::{json, Map};

#[path = "workflow_recommendation_contract.rs"]
mod contract;

use contract::{validate_action, validate_catalog, validate_v1, validate_v2};
pub(crate) use contract::{validate_authority_event_bytes, validate_v2_bytes};

pub(crate) const CATALOG_PATH: &str = "orchestration/workflow-adapters.v1.json";
const REQUEST_SCHEMA: &str = "code-intel-workflow-recommendation-request.v2";
const V1_SCHEMA: &str = "code-intel-advisory-workflow-recommendation.v1";
const V2_SCHEMA: &str = "code-intel-advisory-workflow-recommendation.v2";
const ARTIFACT_TYPE: &str = "advisory.workflow-recommendation";

const INTENTS: [&str; 8] = [
    "explore",
    "plan",
    "implement",
    "verify",
    "archive",
    "synchronize",
    "ship",
    "observe",
];
const CAPABILITIES: [&str; 9] = [
    "delta-governance",
    "continuous-change",
    "constitution",
    "clarification",
    "checklists",
    "convergence",
    "composed-workflow",
    "brownfield-change",
    "bounded-local-work",
];
const ADAPTERS: [&str; 3] = ["openspec", "spec-kit", "lightweight"];

struct Options {
    repo: PathBuf,
    intents: BTreeSet<String>,
    capabilities: BTreeSet<String>,
    preferred: Option<String>,
    override_reason: Option<String>,
}

struct Adoption {
    adapter: String,
    reference: String,
}

struct Presence {
    state: &'static str,
    evidence: Vec<String>,
    active: bool,
}

pub(crate) fn execute_v1(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    if !verified_inputs.is_empty() {
        return Err(AdapterError::Contract(
            "advisory.workflow-recommend does not accept input artifacts".into(),
        ));
    }
    let options = parse_v1_options(request)?;
    let catalog = load_catalog()?;
    let v2 = evaluate(&options, &catalog, None)?;
    let proposal = project_v1(&v2, &options)?;
    validate_v1(&proposal)?;
    emit(out, "workflow-recommendation.json", V1_SCHEMA, &proposal)
}

pub(crate) fn execute_v2(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    let options = parse_v2_options(request)?;
    let adoption = parse_adoption(verified_inputs)?;
    let catalog = load_catalog()?;
    let proposal = evaluate(&options, &catalog, adoption.as_ref())?;
    validate_v2(&proposal)?;
    emit(out, "workflow-recommendation.v2.json", V2_SCHEMA, &proposal)
}

fn emit(
    out: &Path,
    relative: &str,
    schema: &str,
    proposal: &Value,
) -> Result<AdapterOutput, AdapterError> {
    let bytes = serde_json::to_vec(proposal).map_err(|error| {
        AdapterError::Internal(format!("serialize workflow recommendation: {error}"))
    })?;
    publish_named(out, relative, &bytes, |_| Ok(()))?;
    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: schema.into(),
            artifact_type: ARTIFACT_TYPE.into(),
            relative_path: relative.into(),
            bytes,
        }],
        observed_effects: vec![],
        domain_verdict: AdapterDomainVerdict::Pass,
        domain_failure: None,
    })
}

fn parse_v1_options(request: &Value) -> Result<Options, AdapterError> {
    let options = request
        .get("options")
        .and_then(Value::as_object)
        .ok_or_else(|| AdapterError::InvalidOptions("options must be an object".into()))?;
    if options
        .keys()
        .any(|key| !matches!(key.as_str(), "repoPath" | "auto"))
    {
        return Err(AdapterError::InvalidOptions(
            "advisory.workflow-recommend accepts only repoPath/auto".into(),
        ));
    }
    if let Some(auto) = options.get("auto") {
        if !auto.is_boolean() {
            return Err(AdapterError::InvalidOptions(
                "options.auto must be boolean when present".into(),
            ));
        }
    }
    Ok(Options {
        repo: repo_path(options)?,
        intents: BTreeSet::from(["plan".to_string()]),
        capabilities: BTreeSet::new(),
        preferred: None,
        override_reason: None,
    })
}

fn parse_v2_options(request: &Value) -> Result<Options, AdapterError> {
    let options = request
        .get("options")
        .and_then(Value::as_object)
        .ok_or_else(|| AdapterError::InvalidOptions("options must be an object".into()))?;
    let allowed = [
        "repoPath",
        "requestedIntents",
        "requiredCapabilities",
        "preferredAdapter",
        "manualOverrideReason",
    ];
    if options.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(AdapterError::InvalidOptions(format!(
            "{REQUEST_SCHEMA} options are closed"
        )));
    }
    let intents = string_set(
        options.get("requestedIntents"),
        "options.requestedIntents",
        &INTENTS,
    )?;
    let capabilities = string_set(
        options.get("requiredCapabilities"),
        "options.requiredCapabilities",
        &CAPABILITIES,
    )?;
    let preferred = optional_choice(options, "preferredAdapter", &ADAPTERS)?;
    let override_reason = optional_nonempty(options, "manualOverrideReason")?;
    if preferred.is_some() != override_reason.is_some() {
        return Err(AdapterError::InvalidOptions(
            "preferredAdapter and manualOverrideReason must be supplied together".into(),
        ));
    }
    Ok(Options {
        repo: repo_path(options)?,
        intents,
        capabilities,
        preferred,
        override_reason,
    })
}

fn repo_path(options: &Map<String, Value>) -> Result<PathBuf, AdapterError> {
    let repo = options
        .get("repoPath")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| AdapterError::InvalidOptions("options.repoPath must be non-empty".into()))?;
    if !repo.is_dir() {
        return Err(AdapterError::InvalidOptions(format!(
            "repoPath is not a directory: {}",
            repo.display()
        )));
    }
    Ok(repo)
}

fn string_set(
    value: Option<&Value>,
    label: &str,
    allowed: &[&str],
) -> Result<BTreeSet<String>, AdapterError> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| AdapterError::InvalidOptions(format!("{label} must be an array")))?;
    let mut result = BTreeSet::new();
    for value in values {
        let text = value
            .as_str()
            .filter(|item| allowed.contains(item))
            .ok_or_else(|| {
                AdapterError::InvalidOptions(format!("{label} contains an unsupported value"))
            })?;
        if !result.insert(text.to_string()) {
            return Err(AdapterError::InvalidOptions(format!(
                "{label} contains a duplicate value"
            )));
        }
    }
    Ok(result)
}

fn optional_choice(
    options: &Map<String, Value>,
    key: &str,
    allowed: &[&str],
) -> Result<Option<String>, AdapterError> {
    match options.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .filter(|item| allowed.contains(item))
            .map(|item| Some(item.to_string()))
            .ok_or_else(|| AdapterError::InvalidOptions(format!("options.{key} is invalid"))),
    }
}

fn optional_nonempty(
    options: &Map<String, Value>,
    key: &str,
) -> Result<Option<String>, AdapterError> {
    match options.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .filter(|item| !item.trim().is_empty())
            .map(|item| Some(item.to_string()))
            .ok_or_else(|| AdapterError::InvalidOptions(format!("options.{key} is invalid"))),
    }
}

fn parse_adoption(inputs: &[VerifiedArtifact]) -> Result<Option<Adoption>, AdapterError> {
    let [] = inputs else {
        if inputs.len() != 1 {
            return Err(AdapterError::Contract(
                "advisory.workflow-recommend.v2 accepts at most one approved authority event Artifact Ref"
                    .into(),
            ));
        }
        let input = &inputs[0];
        if input.artifact_schema() != "code-intel-authority-event.v1"
            || input.artifact_type() != "authority.event"
        {
            return Err(AdapterError::Contract(
                "workflow adoption evidence must be an A03-verified authority event".into(),
            ));
        }
        let event: Value = serde_json::from_slice(input.bytes()).map_err(|error| {
            AdapterError::Contract(format!("parse workflow adoption authority event: {error}"))
        })?;
        if event["schema"] != "code-intel-authority-event.v1" || event["decision"] != "approved" {
            return Err(AdapterError::Contract(
                "workflow adoption authority event is not approved".into(),
            ));
        }
        let markers = event["evidenceIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|item| item.strip_prefix("workflow-adapter:"))
            .filter(|item| ADAPTERS.contains(item))
            .collect::<BTreeSet<_>>();
        let markers = markers.into_iter().collect::<Vec<_>>();
        let [adapter] = markers.as_slice() else {
            return Err(AdapterError::Contract(
                "authority event must name exactly one workflow-adapter evidence id".into(),
            ));
        };
        return Ok(Some(Adoption {
            adapter: (*adapter).to_string(),
            reference: format!("sha256:{}", input.sha256()),
        }));
    };
    Ok(None)
}

fn load_catalog() -> Result<Vec<Value>, AdapterError> {
    let development = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(CATALOG_PATH);
    let path = if development.is_file() {
        development
    } else {
        super::pipeline_root().join(CATALOG_PATH)
    };
    let bytes = fs::read(&path).map_err(|error| {
        AdapterError::Unavailable(format!(
            "workflow adapter catalog is unavailable: {}: {error}",
            path.display()
        ))
    })?;
    let catalog: Value = serde_json::from_slice(&bytes).map_err(|error| {
        AdapterError::Contract(format!("workflow adapter catalog is not JSON: {error}"))
    })?;
    validate_catalog(&catalog)?;
    Ok(catalog["candidates"].as_array().unwrap().clone())
}

fn evaluate(
    options: &Options,
    catalog: &[Value],
    adoption: Option<&Adoption>,
) -> Result<Value, AdapterError> {
    let candidates = catalog
        .iter()
        .map(|candidate| {
            let adapter = candidate["adapter"].as_str().unwrap();
            (adapter.to_string(), candidate)
        })
        .collect::<BTreeMap<_, _>>();
    let presences = candidates
        .keys()
        .map(|adapter| (adapter.clone(), detect_presence(&options.repo, adapter)))
        .collect::<BTreeMap<_, _>>();
    let active = presences
        .iter()
        .filter(|(_, presence)| presence.active)
        .map(|(adapter, _)| adapter.clone())
        .collect::<Vec<_>>();

    let mut conflict = None;
    let mut selected = None;
    let mut reason = "bounded work has no requested framework capability".to_string();
    if active.len() > 1 {
        conflict = Some(json!({
            "kind":"competing-normative-roots",
            "roots":active.iter().map(|adapter| presences[adapter].evidence[0].clone()).collect::<Vec<_>>(),
            "resolution":"choose one normative source before continuing implementation"
        }));
    } else if let Some(adoption) = adoption {
        selected = Some(adoption.adapter.clone());
        reason = "caller supplied an A03-verified approved authority event".into();
    } else if let Some(adapter) = active.first() {
        selected = Some(adapter.clone());
        reason = "an active normative artifact should be continued".into();
    } else {
        let openspec_fit = has_any(
            &options.capabilities,
            &["delta-governance", "continuous-change"],
        );
        let spec_kit_fit = has_any(
            &options.capabilities,
            &[
                "constitution",
                "clarification",
                "checklists",
                "convergence",
                "composed-workflow",
            ],
        );
        if openspec_fit && spec_kit_fit {
            conflict = Some(json!({
                "kind":"incompatible-required-capabilities",
                "roots":["openspec","spec-kit"],
                "resolution":"split the work or choose which normative model owns this change"
            }));
        } else if openspec_fit {
            selected = Some("openspec".into());
            reason = "required capabilities include delta or continuous-change governance".into();
        } else if spec_kit_fit
            || (options.capabilities.contains("brownfield-change")
                && presences["spec-kit"].state != "absent")
        {
            selected = Some("spec-kit".into());
            reason =
                "required capabilities include constitution, convergence, or composed workflow"
                    .into();
        } else if options.capabilities.contains("brownfield-change") {
            selected = Some("openspec".into());
            reason = "brownfield change is supported by both adapters; OpenSpec delta governance is the default tie-break".into();
        } else if presences["openspec"].state != "absent" && presences["spec-kit"].state == "absent"
        {
            selected = Some("openspec".into());
            reason = "OpenSpec configuration evidence exists".into();
        } else if presences["spec-kit"].state != "absent" && presences["openspec"].state == "absent"
        {
            selected = Some("spec-kit".into());
            reason = "spec-kit configuration evidence exists".into();
        } else {
            selected = Some("lightweight".into());
        }
    }

    let fitted = selected.clone();
    let mut manual_override = None;
    if conflict.is_none() {
        if let (Some(preferred), Some(override_reason)) =
            (&options.preferred, &options.override_reason)
        {
            let from = selected.clone().unwrap_or_else(|| "lightweight".into());
            selected = Some(preferred.clone());
            reason = format!("manual override: {override_reason}");
            manual_override = Some(json!({
                "from":from,
                "to":preferred,
                "reason":override_reason
            }));
        }
    }

    let mut rendered = Vec::new();
    for candidate in catalog {
        let adapter = candidate["adapter"].as_str().unwrap();
        let selected_candidate = selected.as_deref() == Some(adapter) && conflict.is_none();
        let verdict = if selected_candidate && manual_override.is_some() {
            "manual-override"
        } else if selected_candidate && presences[adapter].active {
            "active-continuation"
        } else if selected_candidate {
            "recommended"
        } else {
            "alternative"
        };
        rendered.push(render_candidate(
            candidate,
            &presences[adapter],
            options,
            adoption,
            verdict,
            if selected_candidate { 100 } else { 50 },
            if selected_candidate {
                reason.clone()
            } else {
                "candidate retained for explicit comparison".into()
            },
        )?);
    }
    let recommendation = selected
        .as_ref()
        .filter(|_| conflict.is_none())
        .and_then(|adapter| {
            rendered
                .iter()
                .find(|candidate| candidate["adapter"] == adapter.as_str())
                .cloned()
        });
    let handoffs = options
        .intents
        .iter()
        .filter_map(|intent| match intent.as_str() {
            "ship" => Some(json!({
                "intent":"ship",
                "availability":"unavailable",
                "missingCapability":"tool-neutral-agent-shipping-control-loop"
            })),
            "observe" => Some(json!({
                "intent":"observe",
                "availability":"unavailable",
                "missingCapability":"intervention-outcome-ledger"
            })),
            _ => None,
        })
        .collect::<Vec<_>>();
    let capabilities = if options.capabilities.is_empty() {
        "none".to_string()
    } else {
        options
            .capabilities
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(",")
    };
    let evidence = vec![
        json!({"kind":"required-capabilities","value":capabilities}),
        json!({"kind":"selection-rule","value":reason}),
        json!({
            "kind":"repository-presence",
            "value":presences.iter().map(|(adapter, presence)| format!("{adapter}:{}", presence.state)).collect::<Vec<_>>().join(",")
        }),
    ];
    let source_versions = catalog
        .iter()
        .map(|candidate| candidate["source"].clone())
        .collect::<Vec<_>>();
    Ok(json!({
        "schema":V2_SCHEMA,
        "kind":"proposal",
        "recommendation":recommendation,
        "evidence":evidence,
        "confidence":if conflict.is_some() || adoption.is_some() || active.len() == 1 { "high" } else if fitted.as_deref() == Some("lightweight") { "low" } else { "medium" },
        "alternatives":rendered,
        "provenance":{
            "capabilityId":"advisory.workflow-recommend.v2",
            "implementation":"workflow_recommendation.rs",
            "repository":options.repo.to_string_lossy(),
            "catalog":CATALOG_PATH,
            "sourceVersions":source_versions
        },
        "effects":[],
        "conflict":conflict,
        "manualOverride":manual_override,
        "handoffs":handoffs
    }))
}

fn render_candidate(
    candidate: &Value,
    presence: &Presence,
    options: &Options,
    adoption: Option<&Adoption>,
    verdict: &str,
    score: u64,
    reason: String,
) -> Result<Value, AdapterError> {
    let adapter = candidate["adapter"].as_str().unwrap();
    let entry_actions = candidate["entryActions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| {
            options.intents.is_empty()
                || action["intent"]
                    .as_str()
                    .is_some_and(|intent| options.intents.contains(intent))
        })
        .map(|action| resolve_action(&options.repo, adapter, action))
        .collect::<Result<Vec<_>, _>>()?;
    let setup_actions = candidate["setupActions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|action| resolve_action(&options.repo, adapter, action))
        .collect::<Result<Vec<_>, _>>()?;
    let maintenance_actions = candidate["maintenanceActions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|action| resolve_action(&options.repo, adapter, action))
        .collect::<Result<Vec<_>, _>>()?;
    let approved = adoption.filter(|value| value.adapter == adapter);
    Ok(json!({
        "candidate":candidate["candidate"],
        "stack":candidate["stack"],
        "adapter":adapter,
        "verdict":verdict,
        "score":score,
        "reasons":[reason],
        "presence":{"state":presence.state,"evidence":presence.evidence},
        "adoption":{
            "state":if approved.is_some() { "approved" } else { "unresolved" },
            "authorityEventRef":approved.map(|value| value.reference.clone())
        },
        "entryActions":entry_actions,
        "setupActions":setup_actions,
        "maintenanceActions":maintenance_actions,
        "source":candidate["source"],
        "capabilities":candidate["capabilities"]
    }))
}

fn resolve_action(repo: &Path, adapter: &str, action: &Value) -> Result<Value, AdapterError> {
    let mut action = action.clone();
    if adapter == "lightweight" {
        return Ok(action);
    }
    let generated = action["prerequisites"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|item| item.strip_prefix("generated-action:"))
        .any(|name| generated_action_exists(repo, name));
    if generated {
        action["availability"] = json!("available");
    } else {
        action["availability"] = json!("conditional");
        action["invocations"] = json!({"codex":null,"generic":null,"cli":null});
    }
    validate_action(&action)?;
    Ok(action)
}

fn generated_action_exists(repo: &Path, name: &str) -> bool {
    [".agents/skills", ".codex/skills"]
        .iter()
        .any(|root| repo.join(root).join(name).join("SKILL.md").is_file())
}

fn detect_presence(repo: &Path, adapter: &str) -> Presence {
    match adapter {
        "openspec" => {
            let root = repo.join("openspec");
            let active = child_with_file(
                &root.join("changes"),
                &["proposal.md", "design.md", "tasks.md"],
            );
            let mut evidence = Vec::new();
            if active {
                evidence.push("openspec/changes/<active-change>".into());
            } else if root.is_dir() {
                evidence.push("openspec/".into());
            }
            Presence {
                state: if active {
                    "active"
                } else if root.is_dir() {
                    "configured"
                } else {
                    "absent"
                },
                evidence,
                active,
            }
        }
        "spec-kit" => {
            let configured = repo.join(".specify").is_dir() || repo.join("specs").is_dir();
            let active = child_with_file(&repo.join("specs"), &["spec.md", "tasks.md", "plan.md"]);
            let mut evidence = Vec::new();
            if active {
                evidence.push("specs/<active-feature>".into());
            } else if repo.join(".specify").is_dir() {
                evidence.push(".specify/".into());
            } else if repo.join("specs").is_dir() {
                evidence.push("specs/".into());
            }
            Presence {
                state: if active {
                    "active"
                } else if configured {
                    "configured"
                } else {
                    "absent"
                },
                evidence,
                active,
            }
        }
        _ => Presence {
            state: "absent",
            evidence: Vec::new(),
            active: false,
        },
    }
}

fn child_with_file(root: &Path, names: &[&str]) -> bool {
    let Ok(entries) = fs::read_dir(root) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        entry.path().is_dir()
            && name != "archive"
            && names.iter().any(|file| entry.path().join(file).is_file())
    })
}

fn has_any(values: &BTreeSet<String>, expected: &[&str]) -> bool {
    expected.iter().any(|value| values.contains(*value))
}

fn project_v1(v2: &Value, options: &Options) -> Result<Value, AdapterError> {
    let selected = v2["recommendation"].as_object();
    let (candidate, verdict, score, reasons, entry_skills) = if let Some(selected) = selected {
        let entry_skills = selected["entryActions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|action| action["availability"] == "available")
            .filter_map(|action| {
                action["invocations"]["codex"]
                    .as_str()
                    .or_else(|| action["invocations"]["generic"].as_str())
            })
            .map(str::to_string)
            .collect::<Vec<_>>();
        (
            selected["candidate"].clone(),
            selected["verdict"].clone(),
            selected["score"].clone(),
            selected["reasons"].clone(),
            entry_skills,
        )
    } else {
        (
            json!("lightweight-local"),
            json!("conflict"),
            json!(0),
            json!(["competing normative roots require an explicit resolution"]),
            Vec::new(),
        )
    };
    let brief = json!({
        "recommended":candidate,
        "verdict":verdict,
        "confidence":v2["confidence"],
        "why":reasons,
        "whyNot":["Other adapters remain candidates when their required capabilities apply."],
        "doFirst":["Use the structured adapter entry action that is observed as available."],
        "doNotDoYet":["Do not auto-run init from Code Intel Pipeline.","Do not treat a recommendation as adoption authority."],
        "fallback":"Use bounded local work when no adapter capability is required.",
        "acceptance":["Completion conditions are explicit and verifiable."],
        "sourceMethod":"EternallLight/improving-ai-agent-openspec methodology: requirement coverage, acceptance tests, and done criteria."
    });
    let recommendation = json!({
        "candidate":candidate,
        "stack":"spec-driven",
        "verdict":verdict,
        "score":score,
        "reasons":reasons,
        "entrySkills":entry_skills,
        "brief":brief
    });
    let alternatives = vec![
        json!({"candidate":"matt-flow","stack":"matt-flow","verdict":"candidate","score":50,"reasons":["retained v1 compatibility candidate"],"entrySkills":[]}),
        json!({"candidate":"gstack","stack":"gstack","verdict":"candidate","score":50,"reasons":["retained v1 compatibility candidate"],"entrySkills":[]}),
        recommendation.clone(),
    ];
    Ok(json!({
        "schema":V1_SCHEMA,
        "kind":"proposal",
        "recommendation":recommendation,
        "evidence":v2["evidence"],
        "confidence":v2["confidence"],
        "alternatives":alternatives,
        "provenance":{
            "capabilityId":"advisory.workflow-recommend",
            "implementation":"workflow_recommendation.rs",
            "repository":options.repo.to_string_lossy(),
            "compatibilityOptions":{"auto":false}
        },
        "effects":[]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_vocabularies_are_closed_and_do_not_contain_phrase_aliases() {
        assert_eq!(INTENTS.len(), 8);
        assert_eq!(CAPABILITIES.len(), 9);
        assert!(!INTENTS.contains(&"定案"));
        assert!(!INTENTS.contains(&"开始做"));
    }

    #[test]
    fn catalog_is_closed_and_exactly_pinned() {
        let catalog = load_catalog().unwrap();
        assert_eq!(catalog.len(), 3);
        assert_eq!(catalog[0]["source"]["version"], "1.8.0");
        assert_eq!(
            catalog[0]["source"]["revision"],
            "d57889664cab4f2f061d236ec3ff82a5578701bb"
        );
        assert_eq!(catalog[1]["source"]["version"], "0.16.1");
        assert_eq!(
            catalog[1]["source"]["revision"],
            "ad4104b56c219b0a27bac06547d1a3c7d6a0dbd6"
        );
    }
}
