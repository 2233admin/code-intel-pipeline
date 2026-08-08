//! The six served tools.
//!
//! Each one is a projection: it re-reads what `run commit` published, or it
//! re-runs a registered preview capability. None of them decides anything. The
//! request types come from the same modules the CLI parsers use
//! (`evidence_query`, `change_impact`), so a path arriving as a JSON string
//! over stdio meets the same traversal and inventory guards as one typed after
//! `--changed` — there is no second, looser parser on this side.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

use super::ServeContext;
use crate::committed_evidence::{self, CommittedEvidence, EvidenceError};
use crate::{
    capability, capability_inventory, change_impact, evidence_query, execution_policy, snapshot,
};

const EDIT_PLAN_CAPABILITY: &str = "edit.ast-grep-plan";
const DEFAULT_LIMIT: usize = 20;
/// Read from `evidence_query` rather than restated: the tool descriptors, the
/// `--limit` flag, and this surface must agree on the ceiling, and three copies
/// of `100` is how they stop agreeing.
use crate::evidence_query::MAX_LIMIT;

pub(super) fn call(context: &ServeContext, name: &str, arguments: &Value) -> Result<Value, String> {
    if !arguments.is_object() {
        return Err("arguments must be a JSON object".into());
    }
    match name {
        "get_gate_verdict" => gate_verdict(context, arguments),
        "get_facts" => facts(context, arguments),
        "get_evidence" => evidence_chain(context, arguments),
        "get_audit_status" => audit_status(context, arguments),
        "get_change_impact" => blast_radius(context, arguments),
        "plan_structural_edit" => structural_edit(context, arguments),
        other => Err(format!("unknown tool: {other}")),
    }
}

fn gate_verdict(context: &ServeContext, arguments: &Value) -> Result<Value, String> {
    expect_keys(arguments, &[])?;
    let evidence = load(context)?;
    let freshness = freshness(&evidence, context)?;
    let mut result = json!({
        "schema": "code-intel-mcp-gate-verdict.v1",
        "repo": context.repo,
        "run": evidence.entry["run"],
        "runIdentity": evidence.entry["runIdentity"],
        "runOutcome": evidence.entry["outcome"],
        "snapshotIdentity": evidence.snapshot_identity(),
        "freshness": freshness,
        "minimalRerunCommand": rerun_command(context),
        // Said out loud on every verdict: reading a verdict here is not the
        // same as the gate having run, and this surface cannot make it run.
        "authority": {"status": "committed", "readOnly": true, "gatesHere": false},
    });
    match evidence.artifact("diagnosis.hospital") {
        Some((artifact_ref, verified)) => {
            let hospital: Value = serde_json::from_slice(verified.bytes())
                .map_err(|error| format!("committed hospital artifact is invalid JSON: {error}"))?;
            let triage = &hospital["triage"];
            let failing = triage["failing_rules"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            result["verdict"] = json!({
                "status": triage["status"],
                "domainVerdict": hospital["domainVerdict"],
                "primaryDiagnosis": triage["primary_diagnosis"],
                "disposition": triage["disposition"],
                "risk": hospital["diagnosis"]["risk"],
            });
            result["firstFailingRule"] = failing.first().cloned().unwrap_or(Value::Null);
            result["failingRuleCount"] = json!(failing.len());
            result["evidenceRef"] = artifact_ref.clone();
        }
        None => {
            result["verdict"] = json!({
                "status": "unknown",
                "reason": "the committed run publishes no diagnosis.hospital artifact",
            });
            result["firstFailingRule"] = Value::Null;
            result["failingRuleCount"] = Value::Null;
            result["evidenceRef"] = Value::Null;
        }
    }
    Ok(result)
}

fn facts(context: &ServeContext, arguments: &Value) -> Result<Value, String> {
    expect_keys(arguments, &["type", "artifactSchema", "contains", "limit"])?;
    let request = evidence_query::EvidenceQueryRequest::new(
        context.artifact_root.clone(),
        context.repo.clone(),
        Some(context.repo_path.clone()),
        optional_text(arguments, "artifactSchema")?,
        optional_text(arguments, "type")?,
        optional_text(arguments, "contains")?,
        optional_limit(arguments)?,
    )
    .map_err(query_message)?;
    let evidence = load(context)?;
    evidence_query::execute(request, &evidence)
        .map(|result| result.value().clone())
        .map_err(query_message)
}

/// The provenance chain behind one identifier.
///
/// "Which artifacts mention this" is a weaker claim than "this finding is
/// real", and the result says so: `status` reports whether committed evidence
/// backs the identifier at all, and an empty chain is reported as `unbacked`
/// rather than as an error. An agent asked to justify a finding needs to be
/// able to discover that nothing recorded supports it.
fn evidence_chain(context: &ServeContext, arguments: &Value) -> Result<Value, String> {
    expect_keys(arguments, &["findingId", "limit"])?;
    let finding = required_text(arguments, "findingId")?;
    let limit = optional_limit(arguments)?;
    let evidence = load(context)?;
    let needle = finding.to_lowercase();
    let mut links = Vec::new();
    let mut scanned = 0usize;
    let mut truncated = false;
    for (artifact_ref, verified) in evidence.refs.iter().zip(evidence.verified.iter()) {
        scanned += 1;
        if !String::from_utf8_lossy(verified.bytes())
            .to_lowercase()
            .contains(&needle)
        {
            continue;
        }
        if links.len() == limit {
            truncated = true;
            break;
        }
        links.push(json!({
            "artifactRef": artifact_ref,
            "artifactType": artifact_ref["type"],
            "artifactSchema": artifact_ref["artifactSchema"],
            "sha256": artifact_ref["sha256"],
            "consumedSnapshotIdentity": artifact_ref["consumedSnapshotIdentity"],
        }));
    }
    Ok(json!({
        "schema": "code-intel-mcp-evidence-chain.v1",
        "repo": context.repo,
        "run": evidence.entry["run"],
        "runIdentity": evidence.entry["runIdentity"],
        "snapshotIdentity": evidence.snapshot_identity(),
        "findingId": finding,
        "status": if links.is_empty() { "unbacked" } else { "backed" },
        "links": links,
        "linksTruncated": truncated,
        "artifactsScanned": scanned,
        "explanation": "Each link is an A07-committed artifact whose verified bytes mention the \
    requested identifier. A digest-verified mention is provenance, not proof that the finding is \
    correct; 'unbacked' means no committed artifact mentions it, not that it is false.",
    }))
}

fn audit_status(context: &ServeContext, arguments: &Value) -> Result<Value, String> {
    expect_keys(arguments, &["department"])?;
    let wanted = optional_text(arguments, "department")?;
    let evidence = load(context)?;
    let mut result = json!({
        "schema": "code-intel-mcp-audit-status.v1",
        "repo": context.repo,
        "run": evidence.entry["run"],
        "snapshotIdentity": evidence.snapshot_identity(),
        "department": wanted,
    });
    let Some((artifact_ref, verified)) = evidence.artifact("diagnosis.audit") else {
        // An absent audit is not a clean audit. Naming the reason keeps an
        // agent from reading silence as a pass.
        result["status"] = json!("unavailable");
        result["reason"] = json!(
            "the committed run publishes no diagnosis.audit artifact; no audit department has run \
against this snapshot"
        );
        return Ok(result);
    };
    let report: Value = serde_json::from_slice(verified.bytes())
        .map_err(|error| format!("committed audit artifact is invalid JSON: {error}"))?;
    let matches = |row: &Value, key: &str| match &wanted {
        Some(wanted) => row[key] == json!(wanted),
        None => true,
    };
    let filtered = |key: &str, id_key: &str| -> Vec<Value> {
        report[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|row| matches(row, id_key))
            .cloned()
            .collect()
    };
    let departments = filtered("departments", "id");
    if wanted.is_some() && departments.is_empty() {
        result["status"] = json!("unknown_department");
        result["reason"] = json!("the committed audit report has no run for that department");
        result["knownDepartments"] = json!(report["departments"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| row["id"].as_str())
            .collect::<Vec<_>>());
        return Ok(result);
    }
    result["status"] = json!("available");
    result["departments"] = json!(departments);
    result["scores"] = json!(filtered("scores", "department"));
    result["coverage"] = json!(filtered("coverage", "department"));
    result["findings"] = json!(filtered("findings", "department"));
    result["evidenceRef"] = artifact_ref.clone();
    Ok(result)
}

/// The mid-edit question the CLI refuses by design.
///
/// `change impact` fails closed when the committed snapshot is not current,
/// which is always true while an agent is typing — that refusal is what made
/// the pipeline invisible at write time (#58). Here the default is the
/// advisory answer, labelled with both snapshot identities, and `requireCurrent`
/// restores the strict behaviour for a caller that wants it.
fn blast_radius(context: &ServeContext, arguments: &Value) -> Result<Value, String> {
    expect_keys(arguments, &["changed", "requireCurrent"])?;
    let changed = required_string_array(arguments, "changed")?;
    let require_current = optional_bool(arguments, "requireCurrent")?.unwrap_or(false);
    // The request is built before the evidence is loaded so a rejected path
    // costs an argument check rather than a full index rebuild — and so the
    // traversal guard is reachable in a test that has no committed run.
    let request = change_impact::ChangeImpactRequest::new(
        context.artifact_root.clone(),
        context.repo.clone(),
        context.repo_path.clone(),
        changed,
    )
    .map_err(impact_message)?;
    let evidence = load(context)?;
    let result = if require_current {
        change_impact::execute_committed(request, &evidence)
    } else {
        change_impact::execute_stale_advisory(request, &evidence)
    }
    .map_err(impact_message)?;
    Ok(result.into_value())
}

fn structural_edit(context: &ServeContext, arguments: &Value) -> Result<Value, String> {
    expect_keys(arguments, &["language", "pattern", "rewrite", "paths"])?;
    let language = required_text(arguments, "language")?;
    let pattern = required_text(arguments, "pattern")?;
    let rewrite = optional_text(arguments, "rewrite")?;
    let paths = match arguments.get("paths") {
        None | Some(Value::Null) => vec![".".to_string()],
        Some(_) => required_string_array(arguments, "paths")?,
    };
    let declaration =
        capability::declaration_for(EDIT_PLAN_CAPABILITY, context.manifest.as_deref())
            .map_err(|error| format!("capability registry: {error}"))?;
    let policy =
        execution_policy::ExecutionPolicy::for_profile(execution_policy::RunProfile::Default);
    let allowed_effects = policy.allowed_effects(&declaration);
    refuse_repository_mutation(&allowed_effects)?;
    let snapshot = snapshot::build_for_dag(&context.repo_path, "explicit_overlay", &paths)
        .map_err(|error| format!("snapshot identity for the requested paths: {error}"))?;

    // `rewrite` is inserted only when supplied: the adapter validates its
    // options as a closed key set and rejects a null rewrite as an empty
    // string, so an omitted argument must stay an omitted key.
    let mut options = Map::new();
    options.insert("repoPath".into(), json!(context.repo_path));
    options.insert("language".into(), json!(language));
    options.insert("pattern".into(), json!(pattern));
    if let Some(rewrite) = &rewrite {
        options.insert("rewrite".into(), json!(rewrite));
    }
    options.insert("paths".into(), json!(paths));

    let request = json!({
        "schema": "code-intel-capability-request.v1",
        "capability": EDIT_PLAN_CAPABILITY,
        "contractVersion": 1,
        "implementation": declaration["implementation"],
        "snapshot": snapshot["snapshot"],
        "options": options,
        "inputs": [],
        "effectPolicy": {"allowedEffects": allowed_effects},
    });

    let staging = staging_path()?;
    let outcome = capability::exec_in_process(
        EDIT_PLAN_CAPABILITY,
        &request,
        &staging,
        context.manifest.as_deref(),
        capability_inventory::execute,
    );
    // Each step keeps its own failure reason. Collapsing all three through
    // `.ok()` reported "produced no artifact" for a plan that was produced and
    // then turned out to be unreadable or malformed — three very different
    // things for whoever has to work out why.
    let plan = read_plan_artifact(&outcome, &staging);
    let _ = fs::remove_dir_all(&staging);
    match plan {
        Ok(plan) => Ok(json!({
            "schema": "code-intel-mcp-structural-edit-plan.v1",
            "capability": EDIT_PLAN_CAPABILITY,
            "repo": context.repo,
            "snapshotIdentity": snapshot["snapshot"]["identity"],
            "authority": {"mode": "preview_only", "repositoryMutation": false},
            "exitCode": outcome.exit_code,
            "plan": plan,
        })),
        Err(reason) => Err(format!(
            "{reason} (exit {}): {}",
            outcome.exit_code,
            outcome
                .diagnostic
                .unwrap_or_else(|| "no diagnostic was emitted".into())
        )),
    }
}

fn read_plan_artifact(outcome: &capability::ExecOutcome, staging: &Path) -> Result<Value, String> {
    let relative = outcome
        .result
        .as_ref()
        .and_then(|result| result["artifacts"][0]["path"].as_str())
        .ok_or("edit plan produced no artifact")?;
    let bytes = fs::read(staging.join(relative))
        .map_err(|error| format!("edit plan artifact {relative} could not be read: {error}"))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("edit plan artifact {relative} is not valid JSON: {error}"))
}

/// The read-only boundary, enforced against the registry rather than asserted
/// in prose.
///
/// `plan_structural_edit` is the one tool here that executes anything. If the
/// capability it fronts ever declares `repo_mutation`, that is a contract
/// change someone must review — not something this server should discover at
/// runtime and proceed through. Refusing here means no argument value, however
/// crafted, can reach a writer from the MCP surface.
pub(super) fn refuse_repository_mutation(allowed_effects: &Value) -> Result<(), String> {
    let mutates = allowed_effects
        .as_array()
        .into_iter()
        .flatten()
        .any(|effect| effect == "repo_mutation");
    if mutates {
        return Err(format!(
            "refused: {EDIT_PLAN_CAPABILITY} declares repo_mutation; the MCP surface never \
executes a repository writer"
        ));
    }
    Ok(())
}

fn load(context: &ServeContext) -> Result<CommittedEvidence, String> {
    committed_evidence::load(&context.artifact_root, &context.repo).map_err(|error| match error {
        EvidenceError::Contract(message) | EvidenceError::HostIo(message) => {
            format!("{message}; rerun: {}", rerun_command(context))
        }
    })
}

fn freshness(evidence: &CommittedEvidence, context: &ServeContext) -> Result<Value, String> {
    evidence
        .freshness(Some(&context.repo_path))
        .map_err(|error| match error {
            EvidenceError::Contract(message) | EvidenceError::HostIo(message) => message,
        })
}

/// The command is meant to be run, so it is spelled the way a shell accepts.
///
/// `repo_path` is canonicalized, which on Windows yields the `\\?\` verbatim
/// prefix. Handing an agent a command it cannot paste is worse than handing it
/// none — it will invent one.
pub(super) fn rerun_command(context: &ServeContext) -> String {
    let path = context.repo_path.display().to_string();
    let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
    if path.contains(' ') {
        format!("code-intel \"{path}\" --mode normal")
    } else {
        format!("code-intel {path} --mode normal")
    }
}

fn staging_path() -> Result<PathBuf, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("resolve staging nonce: {error}"))?
        .as_nanos();
    Ok(env::temp_dir().join(format!(
        "code-intel-mcp-plan-{}-{nonce}",
        std::process::id()
    )))
}

/// A closed argument set, checked before anything is read.
///
/// The tool schemas declare `additionalProperties: false`, but a schema is a
/// hint to a well-behaved client, not a guard. Rejecting unexpected keys here
/// means a caller cannot smuggle an argument that a future version of a
/// handler might start honouring.
fn expect_keys(arguments: &Value, allowed: &[&str]) -> Result<(), String> {
    let object = arguments
        .as_object()
        .ok_or_else(|| "arguments must be a JSON object".to_string())?;
    match object.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(unexpected) => Err(format!(
            "unexpected argument: {unexpected} (accepted: {})",
            if allowed.is_empty() {
                "none".to_string()
            } else {
                allowed.join(", ")
            }
        )),
        None => Ok(()),
    }
}

fn required_text(arguments: &Value, key: &str) -> Result<String, String> {
    optional_text(arguments, key)?.ok_or_else(|| format!("{key} is required"))
}

fn optional_text(arguments: &Value, key: &str) -> Result<Option<String>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        Some(Value::String(_)) => Err(format!("{key} must not be empty")),
        Some(_) => Err(format!("{key} must be a string")),
    }
}

fn optional_bool(arguments: &Value, key: &str) -> Result<Option<bool>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!("{key} must be a boolean")),
    }
}

/// The declared `1..=100` bound, enforced here rather than at each call site.
///
/// `get_facts` used to be the only bounded caller, because its limit passes
/// through `EvidenceQueryRequest::new`; `get_evidence` consumed the raw value.
/// A `limit` of 0 then made `evidence_chain` break on its first match and
/// report `status: "unbacked"` — a claim that no committed artifact mentions
/// the identifier — while artifacts that mention it were sitting right there.
/// A bound that only one of two callers honours is not a bound.
///
/// The comparison happens on the `u64` before any cast: `as usize` truncates on
/// a 32-bit target, which would turn an out-of-range value into an accepted one.
fn optional_limit(arguments: &Value) -> Result<usize, String> {
    let out_of_range = || format!("limit must be an integer in 1..={MAX_LIMIT}");
    match arguments.get("limit") {
        None | Some(Value::Null) => Ok(DEFAULT_LIMIT),
        Some(Value::Number(value)) => {
            let value = value.as_u64().ok_or_else(out_of_range)?;
            if !(1..=MAX_LIMIT as u64).contains(&value) {
                return Err(out_of_range());
            }
            Ok(value as usize)
        }
        Some(_) => Err("limit must be an integer".into()),
    }
}

fn required_string_array(arguments: &Value, key: &str) -> Result<Vec<String>, String> {
    let items = arguments
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{key} must be an array of strings"))?;
    if items.is_empty() {
        return Err(format!("{key} must contain at least one entry"));
    }
    items
        .iter()
        .map(|item| {
            item.as_str()
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("{key} entries must be non-empty strings"))
        })
        .collect()
}

fn query_message(error: evidence_query::QueryError) -> String {
    match error {
        evidence_query::QueryError::Contract(message)
        | evidence_query::QueryError::HostIo(message) => message,
    }
}

fn impact_message(error: change_impact::ImpactError) -> String {
    match error {
        change_impact::ImpactError::Contract(message)
        | change_impact::ImpactError::HostIo(message) => message,
    }
}
