use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub(crate) struct Options<'a> {
    pub(crate) action: &'a str,
    pub(crate) mode: &'a str,
    pub(crate) manifest: Option<&'a Path>,
    pub(crate) capability: Option<&'a str>,
    pub(crate) repo: Option<&'a Path>,
    pub(crate) json: bool,
}

pub(crate) fn run(options: &Options<'_>) -> Result<()> {
    let action = normalize_action(options.action)?;
    let mode = normalize_mode(options.mode)?;
    let manifest_path = resolve_manifest_path(options.manifest)?;
    let root = root_for_manifest(&manifest_path)?;
    let manifest = read_json(&manifest_path)?;
    let stages = array_values(&manifest, "stages");
    let integrations = array_values(&manifest, "integrations");

    let mut errors = Vec::new();
    let mut stage_ids = HashSet::new();
    let mut stage_order = HashMap::new();

    for stage in &stages {
        let id = string_field(stage, "id").unwrap_or_default();
        if id.trim().is_empty() {
            errors.push("stage id is empty".to_string());
            continue;
        }
        if !stage_ids.insert(id.clone()) {
            errors.push(format!("duplicate stage id: {id}"));
        }
        stage_order.insert(id, int_field(stage, "order"));
    }

    let mut integration_ids = HashSet::new();
    for integration in &integrations {
        let id = string_field(integration, "id").unwrap_or_default();
        let stage = string_field(integration, "stage").unwrap_or_default();
        let entrypoint = string_field(integration, "entrypoint").unwrap_or_default();
        let capabilities = string_array_field(integration, "capabilities");

        if id.trim().is_empty() {
            errors.push("integration id is empty".to_string());
            continue;
        }
        if !integration_ids.insert(id.clone()) {
            errors.push(format!("duplicate integration id: {id}"));
        }
        if !stage_ids.contains(&stage) {
            errors.push(format!(
                "integration {id} references unknown stage: {stage}"
            ));
        }
        if entrypoint.trim().is_empty() {
            errors.push(format!("integration {id} has no entrypoint"));
        } else if should_validate_entrypoint(&entrypoint) {
            let candidate = root.join(&entrypoint);
            if !candidate.is_file() {
                errors.push(format!("integration {id} entrypoint missing: {entrypoint}"));
            }
        }
        if capabilities.is_empty() {
            errors.push(format!("integration {id} exposes no capabilities"));
        }
    }

    let registry_audit = reconcile_production_registry(&root, &manifest, &integration_ids);
    if registry_audit.enforce {
        errors.extend(registry_audit.findings.iter().cloned());
    }

    let mut selected = integrations
        .iter()
        .filter(|integration| integration_matches_capability(integration, options.capability))
        .cloned()
        .collect::<Vec<_>>();
    selected.sort_by(|left, right| {
        let left_stage = string_field(left, "stage").unwrap_or_default();
        let right_stage = string_field(right, "stage").unwrap_or_default();
        let left_order = stage_order.get(&left_stage).copied().unwrap_or(i64::MAX);
        let right_order = stage_order.get(&right_stage).copied().unwrap_or(i64::MAX);
        left_order
            .cmp(&right_order)
            .then_with(|| string_field(left, "id").cmp(&string_field(right, "id")))
    });

    let plan = selected
        .iter()
        .map(|integration| plan_item(integration, options.repo, &mode))
        .collect::<Vec<_>>();

    let mut sorted_stages = stages.clone();
    sorted_stages.sort_by_key(|stage| int_field(stage, "order"));

    let ok = errors.is_empty();
    let out = serde_json::json!({
        "ok": ok,
        "action": action,
        "manifest": manifest_path,
        "policy": manifest.get("policy").cloned().unwrap_or(Value::Null),
        "errors": errors,
        "registryAudit": registry_audit.output(),
        "stages": sorted_stages,
        "integrations": if action == "Validate" { Vec::<Value>::new() } else { plan.clone() },
        "plan": if action == "Plan" { plan.clone() } else { Vec::<Value>::new() }
    });

    if options.json {
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        print_text(&action, &manifest_path, &out);
    }

    if ok {
        Ok(())
    } else {
        Err("orchestration validation failed".into())
    }
}

fn read_json(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(text.trim_start_matches('\u{feff}'))?)
}

fn resolve_manifest_path(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return absolute_file_path(path);
    }
    let relative = Path::new("orchestration").join("integrations.json");
    for start in manifest_search_starts()? {
        for ancestor in start.ancestors() {
            let candidate = ancestor.join(&relative);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err("orchestration manifest missing: orchestration/integrations.json".into())
}

fn manifest_search_starts() -> Result<Vec<PathBuf>> {
    let mut starts = Vec::new();
    starts.push(env::current_dir()?);
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            starts.push(parent.to_path_buf());
        }
    }
    Ok(starts)
}

fn absolute_file_path(path: &Path) -> Result<PathBuf> {
    if path.is_file() {
        return Ok(if path.is_absolute() {
            path.to_path_buf()
        } else {
            env::current_dir()?.join(path)
        });
    }
    Err(format!("file does not exist: {}", path.display()).into())
}

fn root_for_manifest(manifest_path: &Path) -> Result<PathBuf> {
    let parent = manifest_path
        .parent()
        .ok_or("manifest path has no parent directory")?;
    if parent
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("orchestration"))
    {
        return parent
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| "orchestration manifest has no repository root".into());
    }
    Ok(parent.to_path_buf())
}

fn normalize_action(value: &str) -> Result<String> {
    match value.to_ascii_lowercase().as_str() {
        "validate" => Ok("Validate".to_string()),
        "list" => Ok("List".to_string()),
        "plan" => Ok("Plan".to_string()),
        "audit" => Ok("Audit".to_string()),
        other => Err(format!("unsupported orchestration action: {other}").into()),
    }
}

struct RegistryAudit {
    configured: bool,
    enforce: bool,
    findings: Vec<String>,
    participants: Vec<Value>,
}

impl RegistryAudit {
    fn output(&self) -> Value {
        serde_json::json!({
            "configured": self.configured,
            "mode": if self.enforce { "enforce" } else { "report" },
            "ok": self.findings.is_empty(),
            "findings": self.findings,
            "participants": self.participants,
        })
    }
}

#[derive(Clone, Copy)]
struct ProductionParticipant {
    capability_id: &'static str,
    source: &'static str,
    marker: &'static str,
}

const PRODUCTION_PARTICIPANTS: [ProductionParticipant; 12] = [
    ProductionParticipant {
        capability_id: "doctor",
        source: "invoke-code-intel.ps1",
        marker: "$doctor = Join-Path $root \"check-code-intel-tools.ps1\"",
    },
    ProductionParticipant {
        capability_id: "diagnosis.hospital",
        source: "run-code-intel.ps1",
        marker: "$hospitalReport = New-CodeIntelHospitalReport",
    },
    ProductionParticipant {
        capability_id: "pack.repomix",
        source: "run-code-intel.ps1",
        marker: "$repomixTool = Join-Path $PSScriptRoot \"Invoke-RepomixCodePack.ps1\"",
    },
    ProductionParticipant {
        capability_id: "evidence.native-code",
        source: "run-code-intel.ps1",
        marker: "$codeEvidence = New-CodeEvidenceLayer -RepoPath",
    },
    ProductionParticipant {
        capability_id: "evidence.cocoindex-code",
        source: "run-code-intel.ps1",
        marker: "$adapterConfig = Get-JsonProperty $adapters \"cocoindex-code\" $null",
    },
    ProductionParticipant {
        capability_id: "research.github-solution",
        source: "run-code-intel.ps1",
        marker:
            "$githubResearchScript = Join-Path $PSScriptRoot \"Invoke-GitHubSolutionResearch.ps1\"",
    },
    ProductionParticipant {
        capability_id: "memory.repowise",
        source: "run-code-intel.ps1",
        marker: "$scopedRepowiseScript = Join-Path $PSScriptRoot \"Invoke-ScopedRepowise.ps1\"",
    },
    ProductionParticipant {
        capability_id: "graph.code-intel-understand",
        source: "run-code-intel.ps1",
        marker: "$knowledgeGraph = Join-Path $understandDir \"knowledge-graph.json\"",
    },
    ProductionParticipant {
        capability_id: "structure.sentrux",
        source: "run-code-intel.ps1",
        marker: "$sentruxAgentTool = Join-Path $PSScriptRoot \"Invoke-SentruxAgentTool.ps1\"",
    },
    ProductionParticipant {
        capability_id: "localization.codenexus-lite",
        source: "run-code-intel.ps1",
        marker: "$codeNexusLiteTool = Join-Path $PSScriptRoot \"Invoke-CodeNexusLite.ps1\"",
    },
    ProductionParticipant {
        capability_id: "run.commit",
        source: "run-code-intel.ps1",
        marker: "& $rustCli run commit",
    },
    ProductionParticipant {
        capability_id: "artifact.index-committed-only",
        source: "invoke-code-intel.ps1",
        marker: "$indexer = Join-Path $root \"update-code-intel-index.ps1\"",
    },
];

fn reconcile_production_registry(
    root: &Path,
    manifest: &Value,
    integration_ids: &HashSet<String>,
) -> RegistryAudit {
    let Some(config) = manifest.get("productionRegistry") else {
        return empty_registry_audit();
    };

    let configured_mode = string_field(config, "mode").unwrap_or_default();
    let enforce = configured_mode != "report";
    let production_files = string_array_field(config, "productionFiles");
    let declarations = array_values(config, "participants");
    let mut findings = Vec::new();
    validate_registry_mode(&configured_mode, &mut findings);
    let sources = load_production_sources(root, &production_files, &mut findings);
    let declarations_by_id = index_participant_declarations(&declarations, &mut findings);
    let participant_output = PRODUCTION_PARTICIPANTS
        .into_iter()
        .filter_map(|participant| {
            audit_production_participant(
                participant,
                &production_files,
                &sources,
                &declarations_by_id,
                integration_ids,
                &mut findings,
            )
        })
        .collect();
    audit_unknown_declarations(&declarations, &mut findings);

    RegistryAudit {
        configured: true,
        enforce,
        findings,
        participants: participant_output,
    }
}

fn empty_registry_audit() -> RegistryAudit {
    RegistryAudit {
        configured: false,
        enforce: false,
        findings: Vec::new(),
        participants: Vec::new(),
    }
}

fn validate_registry_mode(configured_mode: &str, findings: &mut Vec<String>) {
    if !matches!(configured_mode, "report" | "enforce") {
        findings.push(format!(
            "production registry has unsupported mode: {configured_mode}"
        ));
    }
}

fn load_production_sources(
    root: &Path,
    production_files: &[String],
    findings: &mut Vec<String>,
) -> HashMap<String, String> {
    let mut sources = HashMap::new();
    for relative in production_files {
        match fs::read_to_string(root.join(relative)) {
            Ok(text) => record_production_source(&mut sources, relative, text, findings),
            Err(error) => findings.push(format!(
                "production registry source missing or unreadable: {relative}: {error}"
            )),
        }
    }
    sources
}

fn record_production_source(
    sources: &mut HashMap<String, String>,
    relative: &str,
    text: String,
    findings: &mut Vec<String>,
) {
    if sources.insert(relative.to_string(), text).is_some() {
        findings.push(format!(
            "production registry lists source more than once: {relative}"
        ));
    }
}

fn index_participant_declarations<'a>(
    declarations: &'a [Value],
    findings: &mut Vec<String>,
) -> HashMap<String, &'a Value> {
    let mut declarations_by_id = HashMap::new();
    for declaration in declarations {
        let id = string_field(declaration, "capabilityId").unwrap_or_default();
        if id.trim().is_empty() {
            findings.push("production participant declaration has empty capabilityId".to_string());
        } else if declarations_by_id.insert(id.clone(), declaration).is_some() {
            findings.push(format!(
                "duplicate production participant declaration: {id}"
            ));
        }
    }
    declarations_by_id
}

fn audit_production_participant(
    participant: ProductionParticipant,
    production_files: &[String],
    sources: &HashMap<String, String>,
    declarations_by_id: &HashMap<String, &Value>,
    integration_ids: &HashSet<String>,
    findings: &mut Vec<String>,
) -> Option<Value> {
    audit_participant_source(participant, production_files, findings);
    let hits = sources
        .get(participant.source)
        .map(|text| production_call_sites(participant.source, text, participant.marker))
        .unwrap_or_default();
    let Some(declaration) = declarations_by_id.get(participant.capability_id) else {
        audit_missing_declaration(participant, &hits, findings);
        return None;
    };
    let status = string_field(declaration, "status").unwrap_or_default();
    audit_participant_lifecycle(
        declaration,
        participant,
        &status,
        &hits,
        integration_ids,
        findings,
    );
    Some(participant_audit_output(participant, status, hits))
}

fn audit_participant_source(
    participant: ProductionParticipant,
    production_files: &[String],
    findings: &mut Vec<String>,
) {
    if !production_files
        .iter()
        .any(|path| path == participant.source)
    {
        findings.push(format!(
            "production participant {} source is not audited: {}",
            participant.capability_id, participant.source
        ));
    }
}

fn audit_missing_declaration(
    participant: ProductionParticipant,
    hits: &[String],
    findings: &mut Vec<String>,
) {
    if hits.is_empty() {
        findings.push(format!(
            "required production participant has no declaration: {}",
            participant.capability_id
        ));
    } else {
        findings.push(format!(
            "undeclared production invocation: {} marker '{}'",
            participant.capability_id, participant.marker
        ));
    }
}

fn audit_participant_lifecycle(
    declaration: &Value,
    participant: ProductionParticipant,
    status: &str,
    hits: &[String],
    integration_ids: &HashSet<String>,
    findings: &mut Vec<String>,
) {
    match status {
        "deleted" => {
            audit_deleted_participant(declaration, participant, hits, integration_ids, findings)
        }
        "declared" => {
            audit_declared_participant(declaration, participant, hits, integration_ids, findings)
        }
        _ => findings.push(format!(
            "production participant {} has unsupported status: {status}",
            participant.capability_id
        )),
    }
}

fn audit_deleted_participant(
    declaration: &Value,
    participant: ProductionParticipant,
    hits: &[String],
    integration_ids: &HashSet<String>,
    findings: &mut Vec<String>,
) {
    let capability_id = participant.capability_id;
    let deletion = declaration.get("reviewedDeletion").unwrap_or(&Value::Null);
    for field in ["reviewer", "reviewedAt", "evidence"] {
        if string_field(deletion, field).is_none_or(|value| value.trim().is_empty()) {
            findings.push(format!(
                "reviewed deletion for {capability_id} is missing {field}"
            ));
        }
    }
    if !hits.is_empty() {
        findings.push(format!(
            "deleted production participant is still invoked: {capability_id}"
        ));
    }
    if integration_ids.contains(capability_id) {
        findings.push(format!(
            "deleted production participant remains in integrations registry: {capability_id}"
        ));
    }
    require_call_site_declaration(declaration, participant, findings);
}

fn audit_declared_participant(
    declaration: &Value,
    participant: ProductionParticipant,
    hits: &[String],
    integration_ids: &HashSet<String>,
    findings: &mut Vec<String>,
) {
    let capability_id = participant.capability_id;
    if !integration_ids.contains(capability_id) {
        findings.push(format!(
            "production participant is not in integrations registry: {capability_id}"
        ));
    }
    audit_declared_scalar_metadata(declaration, capability_id, findings);
    audit_declared_array_metadata(declaration, capability_id, findings);
    audit_declared_dependencies(declaration, capability_id, integration_ids, findings);
    audit_declared_artifacts(declaration, capability_id, findings);
    audit_declared_call_sites(participant, hits, findings);
    require_call_site_declaration(declaration, participant, findings);
}

fn audit_declared_scalar_metadata(
    declaration: &Value,
    capability_id: &str,
    findings: &mut Vec<String>,
) {
    for field in ["envelope", "owner"] {
        if string_field(declaration, field).is_none_or(|value| value.trim().is_empty()) {
            findings.push(format!(
                "production participant {capability_id} is missing {field} metadata"
            ));
        }
    }
}

fn audit_declared_array_metadata(
    declaration: &Value,
    capability_id: &str,
    findings: &mut Vec<String>,
) {
    for field in ["dependencies", "effects", "artifacts"] {
        if !is_string_array(declaration, field) {
            findings.push(format!(
                "production participant {capability_id} is missing {field} metadata"
            ));
        }
    }
}

fn audit_declared_dependencies(
    declaration: &Value,
    capability_id: &str,
    integration_ids: &HashSet<String>,
    findings: &mut Vec<String>,
) {
    for dependency in string_array_field(declaration, "dependencies") {
        if !integration_ids.contains(&dependency) {
            findings.push(format!(
                "production participant {capability_id} references unknown dependency: {dependency}"
            ));
        }
    }
}

fn audit_declared_artifacts(declaration: &Value, capability_id: &str, findings: &mut Vec<String>) {
    if string_array_field(declaration, "artifacts").is_empty() {
        findings.push(format!(
            "production participant {capability_id} has empty artifacts metadata"
        ));
    }
}

fn audit_declared_call_sites(
    participant: ProductionParticipant,
    hits: &[String],
    findings: &mut Vec<String>,
) {
    if hits.is_empty() {
        findings.push(format!(
            "orphan production declaration: {} marker '{}' not found",
            participant.capability_id, participant.marker
        ));
    } else if hits.len() != 1 {
        findings.push(format!(
            "production participant {} must have exactly one call site, found {}",
            participant.capability_id,
            hits.len()
        ));
    }
}

fn participant_audit_output(
    participant: ProductionParticipant,
    status: String,
    hits: Vec<String>,
) -> Value {
    serde_json::json!({
        "capabilityId": participant.capability_id,
        "status": status,
        "source": participant.source,
        "marker": participant.marker,
        "callSites": hits,
    })
}

fn audit_unknown_declarations(declarations: &[Value], findings: &mut Vec<String>) {
    for declaration in declarations {
        let id = string_field(declaration, "capabilityId").unwrap_or_default();
        if !PRODUCTION_PARTICIPANTS
            .iter()
            .any(|participant| participant.capability_id == id)
        {
            findings.push(format!(
                "orphan production declaration: unknown capability {id}"
            ));
        }
    }
}

fn production_call_sites(source: &str, text: &str, marker: &str) -> Vec<String> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| line.contains(marker))
        .map(|(index, _)| format!("{source}:{}", index + 1))
        .collect()
}

fn require_call_site_declaration(
    declaration: &Value,
    participant: ProductionParticipant,
    findings: &mut Vec<String>,
) {
    let capability_id = participant.capability_id;
    let Some(call_site) = declaration.get("callSite") else {
        findings.push(format!(
            "production participant {capability_id} is missing callSite metadata"
        ));
        return;
    };
    let declared_source = string_field(call_site, "source").unwrap_or_default();
    let declared_anchor = string_field(call_site, "anchor").unwrap_or_default();
    if declared_source != participant.source || declared_anchor != participant.marker {
        findings.push(format!(
            "production participant {capability_id} callSite contract drift"
        ));
    }
}

fn is_string_array(value: &Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().all(Value::is_string))
}

fn normalize_mode(value: &str) -> Result<String> {
    match value.to_ascii_lowercase().as_str() {
        "lite" => Ok("lite".to_string()),
        "normal" => Ok("normal".to_string()),
        "full" => Ok("full".to_string()),
        other => Err(format!("unsupported orchestration mode: {other}").into()),
    }
}

fn should_validate_entrypoint(entrypoint: &str) -> bool {
    let lower = entrypoint.to_ascii_lowercase();
    [".ps1", ".py", ".toml", ".rs"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
}

fn array_values(value: &Value, key: &str) -> Vec<Value> {
    match value.get(key).and_then(Value::as_array) {
        Some(items) => items.clone(),
        None => Vec::new(),
    }
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn int_field(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn bool_field(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn string_array_field(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn integration_matches_capability(integration: &Value, capability: Option<&str>) -> bool {
    let Some(capability) = capability.filter(|value| !value.trim().is_empty()) else {
        return true;
    };
    string_field(integration, "id").as_deref() == Some(capability)
        || string_field(integration, "stage").as_deref() == Some(capability)
        || string_array_field(integration, "capabilities")
            .iter()
            .any(|item| item == capability)
}

fn plan_item(integration: &Value, repo: Option<&Path>, mode: &str) -> Value {
    let commands = integration
        .get("commands")
        .and_then(Value::as_object)
        .map(|items| {
            let mut expanded = serde_json::Map::new();
            for (name, value) in items {
                if let Some(template) = value.as_str() {
                    expanded.insert(
                        name.clone(),
                        Value::String(expand_command_template(template, repo, mode)),
                    );
                }
            }
            Value::Object(expanded)
        })
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

    serde_json::json!({
        "id": string_field(integration, "id").unwrap_or_default(),
        "stage": string_field(integration, "stage").unwrap_or_default(),
        "kind": string_field(integration, "kind").unwrap_or_default(),
        "required": bool_field(integration, "required"),
        "entrypoint": string_field(integration, "entrypoint").unwrap_or_default(),
        "capabilities": string_array_field(integration, "capabilities"),
        "commands": commands,
        "artifactContract": string_array_field(integration, "artifactContract"),
        "extensionPoint": string_field(integration, "extensionPoint").unwrap_or_default()
    })
}

fn expand_command_template(template: &str, repo: Option<&Path>, mode: &str) -> String {
    let mut expanded = template.replace("<mode>", mode);
    if let Some(repo) = repo {
        expanded = expanded.replace("<repo-path>", &repo.display().to_string());
    }
    expanded
}

fn print_text(action: &str, manifest: &Path, result: &Value) {
    let ok = result.get("ok").and_then(Value::as_bool).unwrap_or(false);
    if !ok {
        println!("Code Intel orchestration: FAILED");
        if let Some(errors) = result.get("errors").and_then(Value::as_array) {
            for error in errors.iter().filter_map(Value::as_str) {
                println!("- {error}");
            }
        }
        return;
    }

    if action == "Validate" {
        let stages = result
            .get("stages")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let integrations = read_manifest_integration_count(result).unwrap_or(0);
        println!("Code Intel orchestration: OK");
        println!("Manifest: {}", manifest.display());
        println!("Stages: {stages}");
        println!("Integrations: {integrations}");
        return;
    }

    println!("Code Intel orchestration: {action}");
    if let Some(items) = result.get("integrations").and_then(Value::as_array) {
        for item in items {
            println!(
                "{}: {} [{}] entry={}",
                string_field(item, "stage").unwrap_or_default(),
                string_field(item, "id").unwrap_or_default(),
                string_field(item, "kind").unwrap_or_default(),
                string_field(item, "entrypoint").unwrap_or_default()
            );
            if action == "Plan" {
                if let Some(commands) = item.get("commands").and_then(Value::as_object) {
                    for (name, value) in commands {
                        if let Some(command) = value.as_str() {
                            println!("  {name}: {command}");
                        }
                    }
                }
            }
        }
    }
}

fn read_manifest_integration_count(result: &Value) -> Option<usize> {
    let manifest = result.get("manifest")?.as_str()?;
    let data = read_json(Path::new(manifest)).ok()?;
    data.get("integrations")
        .and_then(Value::as_array)
        .map(Vec::len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        env::temp_dir().join(format!("code-intel-{name}-{stamp}"))
    }

    fn touch(path: &Path, text: &str) {
        fs::write(path, text).expect("fixture file should be writable");
    }

    fn orchestration_fixture_dir(name: &str) -> PathBuf {
        let dir = unique_temp_dir(name);
        fs::create_dir_all(dir.join("orchestration")).expect("fixture orchestration dir");
        fs::create_dir_all(dir.join("crates/code-intel-cli")).expect("fixture crate dir");
        touch(&dir.join("crates/code-intel-cli/Cargo.toml"), "[package]\n");
        dir
    }

    #[test]
    fn validates_manifest_and_rust_entrypoint() {
        let dir = orchestration_fixture_dir("orchestration-valid");
        let manifest = dir.join("orchestration/integrations.json");
        touch(
            &manifest,
            &json!({
                "schemaVersion": 1,
                "policy": {"name": "test"},
                "stages": [
                    {"id": "rust_runtime", "order": 10, "required": true}
                ],
                "integrations": [
                    {
                        "id": "runtime.code-intel",
                        "stage": "rust_runtime",
                        "kind": "internal-rust-binary",
                        "required": true,
                        "entrypoint": "crates/code-intel-cli/Cargo.toml",
                        "capabilities": ["orchestration"],
                        "commands": {
                            "validate": "target/debug/code-intel.exe orchestrate --action Validate --json",
                            "plan": "target/debug/code-intel.exe orchestrate --action Plan --repo <repo-path> --mode <mode> --json"
                        },
                        "artifactContract": ["integrations.json"]
                    }
                ]
            })
            .to_string(),
        );

        let options = Options {
            manifest: Some(&manifest),
            repo: Some(Path::new("D:/work/demo")),
            action: "Plan",
            mode: "normal",
            capability: Some("orchestration"),
            json: true,
        };

        run(&options).expect("valid orchestration manifest should pass");
    }

    #[test]
    fn fails_when_registered_entrypoint_is_missing() {
        let dir = orchestration_fixture_dir("orchestration-missing");
        let manifest = dir.join("orchestration/integrations.json");
        touch(
            &manifest,
            &json!({
                "schemaVersion": 1,
                "policy": {"name": "test"},
                "stages": [
                    {"id": "rust_runtime", "order": 10, "required": true}
                ],
                "integrations": [
                    {
                        "id": "runtime.missing",
                        "stage": "rust_runtime",
                        "kind": "internal-rust-binary",
                        "required": true,
                        "entrypoint": "crates/missing/Cargo.toml",
                        "capabilities": ["orchestration"],
                        "commands": {},
                        "artifactContract": []
                    }
                ]
            })
            .to_string(),
        );

        let options = Options {
            manifest: Some(&manifest),
            action: "Validate",
            mode: "normal",
            repo: None,
            capability: None,
            json: true,
        };

        let err = run(&options).expect_err("missing entrypoint should fail");
        assert!(err.to_string().contains("orchestration validation failed"));
    }

    #[test]
    fn registry_audit_rejects_undeclared_repomix_invocation() {
        let dir = orchestration_fixture_dir("registry-undeclared-repomix");
        touch(
            &dir.join("run-code-intel.ps1"),
            "$repomixTool = Join-Path $PSScriptRoot \"Invoke-RepomixCodePack.ps1\"\n",
        );
        let manifest = json!({
            "productionRegistry": {
                "mode": "enforce",
                "productionFiles": ["run-code-intel.ps1"],
                "participants": []
            }
        });

        let audit = reconcile_production_registry(&dir, &manifest, &HashSet::new());

        assert!(audit
            .findings
            .iter()
            .any(|finding| finding.contains("undeclared production invocation: pack.repomix")));
    }

    #[test]
    fn registry_audit_rejects_registered_participant_without_dependency_or_effect_metadata() {
        let dir = orchestration_fixture_dir("registry-incomplete-repomix");
        touch(
            &dir.join("run-code-intel.ps1"),
            "$repomixTool = Join-Path $PSScriptRoot \"Invoke-RepomixCodePack.ps1\"\n",
        );
        let manifest = json!({
            "productionRegistry": {
                "mode": "enforce",
                "productionFiles": ["run-code-intel.ps1"],
                "participants": [{
                    "capabilityId": "pack.repomix",
                    "status": "declared",
                    "callSite": {
                        "source": "run-code-intel.ps1",
                        "anchor": "$repomixTool = Join-Path $PSScriptRoot \"Invoke-RepomixCodePack.ps1\""
                    },
                    "envelope": "code-intel-capability-envelope.v1",
                    "owner": "code-intel-pipeline",
                    "artifacts": ["repomix-output.*"]
                }]
            }
        });
        let ids = HashSet::from(["pack.repomix".to_string()]);

        let audit = reconcile_production_registry(&dir, &manifest, &ids);

        assert!(audit
            .findings
            .iter()
            .any(|finding| finding.contains("pack.repomix is missing dependencies metadata")));
        assert!(audit
            .findings
            .iter()
            .any(|finding| finding.contains("pack.repomix is missing effects metadata")));
    }

    #[test]
    fn registry_audit_rejects_duplicate_call_site_and_contract_anchor_drift() {
        let dir = orchestration_fixture_dir("registry-call-site-drift");
        let anchor = "$repomixTool = Join-Path $PSScriptRoot \"Invoke-RepomixCodePack.ps1\"";
        touch(
            &dir.join("run-code-intel.ps1"),
            &format!("{anchor}\n{anchor}\n"),
        );
        let manifest = json!({
            "productionRegistry": {
                "mode": "enforce",
                "productionFiles": ["run-code-intel.ps1"],
                "participants": [{
                    "capabilityId": "pack.repomix",
                    "status": "declared",
                    "callSite": {"source": "run-code-intel.ps1", "anchor": "wrong"},
                    "envelope": "code-intel-capability-envelope.v1",
                    "owner": "code-intel-pipeline",
                    "dependencies": [],
                    "effects": [],
                    "artifacts": ["repomix-summary.json"]
                }]
            }
        });
        let ids = HashSet::from(["pack.repomix".to_string()]);

        let audit = reconcile_production_registry(&dir, &manifest, &ids);

        assert!(audit.findings.iter().any(|finding| {
            finding.contains("pack.repomix must have exactly one call site, found 2")
        }));
        assert!(audit
            .findings
            .iter()
            .any(|finding| finding.contains("pack.repomix callSite contract drift")));
    }

    #[test]
    fn registry_audit_rejects_every_required_participant_metadata_field() {
        let dir = orchestration_fixture_dir("registry-required-metadata");
        let anchor = "$repomixTool = Join-Path $PSScriptRoot \"Invoke-RepomixCodePack.ps1\"";
        touch(&dir.join("run-code-intel.ps1"), &format!("{anchor}\n"));
        let ids = HashSet::from(["pack.repomix".to_string()]);

        for field in [
            "callSite",
            "envelope",
            "owner",
            "dependencies",
            "effects",
            "artifacts",
        ] {
            let mut declaration = json!({
                "capabilityId": "pack.repomix",
                "status": "declared",
                "callSite": {"source": "run-code-intel.ps1", "anchor": anchor},
                "envelope": "code-intel-capability-envelope.v1",
                "owner": "code-intel-pipeline",
                "dependencies": [],
                "effects": [],
                "artifacts": ["repomix-summary.json"]
            });
            declaration.as_object_mut().unwrap().remove(field);
            let manifest = json!({
                "productionRegistry": {
                    "mode": "enforce",
                    "productionFiles": ["run-code-intel.ps1"],
                    "participants": [declaration]
                }
            });

            let audit = reconcile_production_registry(&dir, &manifest, &ids);

            assert!(
                audit
                    .findings
                    .iter()
                    .any(|finding| { finding.contains("pack.repomix") && finding.contains(field) }),
                "missing {field} must fail closed: {:?}",
                audit.findings
            );
        }
    }

    #[test]
    fn registry_audit_report_and_enforce_modes_are_explicit() {
        let dir = orchestration_fixture_dir("registry-modes");
        touch(&dir.join("run-code-intel.ps1"), "# no production calls\n");
        let mut manifest = json!({
            "productionRegistry": {
                "mode": "report",
                "productionFiles": ["run-code-intel.ps1"],
                "participants": []
            }
        });

        let report = reconcile_production_registry(&dir, &manifest, &HashSet::new());
        assert!(!report.enforce);
        assert!(!report.findings.is_empty());

        manifest["productionRegistry"]["mode"] = Value::String("enforce".to_string());
        let enforce = reconcile_production_registry(&dir, &manifest, &HashSet::new());
        assert!(enforce.enforce);
        assert_eq!(report.findings, enforce.findings);
    }

    #[test]
    fn registry_audit_rejects_unknown_orphan_declaration() {
        let dir = orchestration_fixture_dir("registry-orphan");
        touch(&dir.join("run-code-intel.ps1"), "# no production calls\n");
        let manifest = json!({
            "productionRegistry": {
                "mode": "enforce",
                "productionFiles": ["run-code-intel.ps1"],
                "participants": [{"capabilityId": "unknown.future-tool", "status": "declared"}]
            }
        });

        let audit = reconcile_production_registry(&dir, &manifest, &HashSet::new());

        assert!(audit.findings.iter().any(|finding| {
            finding
                .contains("orphan production declaration: unknown capability unknown.future-tool")
        }));
    }

    #[test]
    fn registry_audit_accepts_reviewed_deletion_only_after_call_site_is_removed() {
        let dir = orchestration_fixture_dir("registry-reviewed-deletion");
        touch(&dir.join("run-code-intel.ps1"), "# call site removed\n");
        let anchor = "$repomixTool = Join-Path $PSScriptRoot \"Invoke-RepomixCodePack.ps1\"";
        let manifest = json!({
            "productionRegistry": {
                "mode": "enforce",
                "productionFiles": ["run-code-intel.ps1"],
                "participants": [{
                    "capabilityId": "pack.repomix",
                    "status": "deleted",
                    "callSite": {"source": "run-code-intel.ps1", "anchor": anchor},
                    "reviewedDeletion": {
                        "reviewer": "verifier",
                        "reviewedAt": "2026-07-13T00:00:00Z",
                        "evidence": "fixture:call-site-removed"
                    }
                }]
            }
        });

        let audit = reconcile_production_registry(&dir, &manifest, &HashSet::new());

        assert!(
            audit
                .findings
                .iter()
                .all(|finding| !finding.contains("pack.repomix")),
            "reviewed deletion should close the participant exactly: {:?}",
            audit.findings
        );
    }

    #[test]
    fn checked_in_production_registry_reconciles_eleven_calls_and_one_reviewed_deletion() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let manifest = read_json(&root.join("orchestration/integrations.json")).unwrap();
        let ids = array_values(&manifest, "integrations")
            .iter()
            .filter_map(|integration| string_field(integration, "id"))
            .collect::<HashSet<_>>();

        let audit = reconcile_production_registry(&root, &manifest, &ids);

        assert!(audit.configured);
        assert!(audit.enforce);
        assert!(audit.findings.is_empty(), "{:?}", audit.findings);
        assert_eq!(audit.participants.len(), 12);
        for participant in audit.participants {
            let expected = if participant["status"] == "deleted" {
                0
            } else {
                1
            };
            assert_eq!(
                participant["callSites"].as_array().unwrap().len(),
                expected,
                "{} must reconcile to its declared lifecycle",
                participant["capabilityId"]
            );
        }
    }
}
