use crate::{graph, Result};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

mod evidence;

pub struct Options<'a> {
    pub action: &'a str,
    pub provider: Option<&'a str>,
    pub operation: Option<&'a str>,
    pub repo: Option<&'a Path>,
    pub language: &'a str,
    pub full: bool,
    pub write: bool,
    pub json: bool,
    pub request: Option<&'a Path>,
    pub artifact_root: Option<&'a Path>,
    pub evaluated_at: Option<i64>,
    pub max_age_seconds: Option<i64>,
}

#[derive(Clone, Copy)]
pub struct ProviderOperation {
    pub provider: &'static str,
    pub operation: &'static str,
    pub stage: &'static str,
    pub protocol: &'static str,
    pub method: &'static str,
    pub route: &'static str,
    pub command_template: &'static str,
    pub artifact: &'static str,
    pub required: bool,
    pub status: &'static str,
    pub source_spec: &'static str,
    pub source_identity: Option<&'static str>,
    pub notes: &'static str,
}

pub const OPERATIONS: &[ProviderOperation] = &[
    ProviderOperation {
        provider: "repowise",
        operation: "status",
        stage: "semantic_memory",
        protocol: "cli+mcp-compatible",
        method: "POST",
        route: "/api/providers/repowise/status",
        command_template: "repowise status --no-workspace <repo-path>",
        artifact: ".repowise/state.json",
        required: true,
        status: "active",
        source_spec: "Repowise CLI: status [PATH], MCP/HTTP serve surfaces for agent callers",
        source_identity: None,
        notes: "No model required; reports wiki sync and page statistics.",
    },
    ProviderOperation {
        provider: "repowise",
        operation: "index",
        stage: "semantic_memory",
        protocol: "cli+mcp-compatible",
        method: "POST",
        route: "/api/providers/repowise/index",
        command_template:
            "repowise update --no-workspace --index-only <repo-path> or repowise init --index-only <repo-path>",
        artifact: ".repowise/wiki.db",
        required: true,
        status: "active",
        source_spec: "Repowise CLI: init/update --index-only, MCP tools include semantic code retrieval",
        source_identity: None,
        notes: "No model required; refreshes index, dependency graph, git/dead-code artifacts.",
    },
    ProviderOperation {
        provider: "repowise",
        operation: "docs",
        stage: "semantic_memory",
        protocol: "cli+mcp-compatible",
        method: "POST",
        route: "/api/providers/repowise/docs",
        command_template: "repowise update --docs --no-workspace <repo-path>",
        artifact: "report.json.steps[repowise docs]",
        required: false,
        status: "compatibility",
        source_spec: "Repowise CLI: update --docs, provider/model options",
        source_identity: None,
        notes: "Model-backed; provider quota can disable this without disabling status/index.",
    },
    ProviderOperation {
        provider: "repowise",
        operation: "lite",
        stage: "localization",
        protocol: "http+iii-worker",
        method: "POST",
        route: "/api/providers/repowise/lite",
        command_template: "target/debug/code-nexus-lite.exe codenexus::lite",
        artifact: "codenexus-context.json",
        required: true,
        status: "active",
        source_spec: "CodeNexus-lite worker reads Repowise wiki.db into compact agent context",
        source_identity: None,
        notes: "Agent localization view over the Repowise artifact contract.",
    },
    ProviderOperation {
        provider: "understand",
        operation: "graph",
        stage: "architecture_graph",
        protocol: "artifact+command",
        method: "POST",
        route: "/api/providers/understand/graph",
        command_template:
            "target/debug/code-intel.exe provider --action Invoke --provider understand --operation graph --repo <repo-path> --language zh --write --json",
        artifact: ".understand-anything/knowledge-graph.json",
        required: true,
        status: "active",
        source_spec: "Understand Anything command contract: /understand <repo> --language <lang>, graph artifact",
        source_identity: None,
        notes: "Internal Rust provider emits the Understand-compatible artifact first.",
    },
    ProviderOperation {
        provider: "understand",
        operation: "graph_full",
        stage: "architecture_graph",
        protocol: "artifact+command",
        method: "POST",
        route: "/api/providers/understand/graph/full",
        command_template:
            "target/debug/code-intel.exe provider --action Invoke --provider understand --operation graph_full --repo <repo-path> --language zh --write --json",
        artifact: ".understand-anything/knowledge-graph.json",
        required: false,
        status: "active",
        source_spec: "Understand Anything full graph refresh semantics",
        source_identity: None,
        notes: "Use when a fresh complete graph is requested.",
    },
    ProviderOperation {
        provider: "understand",
        operation: "external_fallback",
        stage: "architecture_graph",
        protocol: "manual-command",
        method: "MANUAL",
        route: "/compat/providers/understand/external",
        command_template: "/understand <repo-path> --language zh",
        artifact: ".understand-anything/knowledge-graph.json",
        required: false,
        status: "compatibility",
        source_spec: "Understand Anything upstream/plugin command surface",
        source_identity: None,
        notes: "Use only if internal graph provider fails or richer external pass is explicitly requested.",
    },
    ProviderOperation {
        provider: "compete",
        operation: "prepare",
        stage: "advisory_evidence",
        protocol: "artifact+agent",
        method: "POST",
        route: "/api/providers/compete/prepare",
        command_template: "Invoke-EvidenceProvider.ps1 -Provider compete -Operation prepare -RepoPath <repo-path> -ArtifactDir <artifact-dir>",
        artifact: "compete-request.json",
        required: false,
        status: "on_demand",
        source_spec: "Compete Agent/web workflow produces schema-marked InsightKit datasets and reports",
        source_identity: Some("MIT revision ec13028fc8da620c73a114ffe403a772b29a78cb"),
        notes: "Prepares an external Claude Code web task; never runs in normal/full pipeline modes.",
    },
    ProviderOperation {
        provider: "compete",
        operation: "status",
        stage: "advisory_evidence",
        protocol: "artifact",
        method: "GET",
        route: "/api/providers/compete/status",
        command_template: "Invoke-EvidenceProvider.ps1 -Provider compete -Operation status -RepoPath <repo-path> -ArtifactDir <artifact-dir>",
        artifact: "compete-native-result.json",
        required: false,
        status: "on_demand",
        source_spec: "Compete artifact status without invoking an Agent",
        source_identity: Some("MIT revision ec13028fc8da620c73a114ffe403a772b29a78cb"),
        notes: "Reports prepared, ready_for_adapt, adapted, or not_run.",
    },
    ProviderOperation {
        provider: "compete",
        operation: "adapt",
        stage: "advisory_evidence",
        protocol: "artifact+command",
        method: "POST",
        route: "/api/providers/compete/adapt",
        command_template: "target/debug/code-intel.exe provider compete-adapt --request <native.json|-> --artifact-root <artifact-dir> --evaluated-at <unix> --max-age-seconds <n>",
        artifact: "code-intel-evidence-route-result.v1",
        required: false,
        status: "active",
        source_spec: "Code Intel A04 advisory evidence admission over Compete native results",
        source_identity: Some("MIT revision ec13028fc8da620c73a114ffe403a772b29a78cb"),
        notes: "Only complete, fresh, snapshot-bound output becomes observed advisory evidence.",
    },
    ProviderOperation {
        provider: "react-doctor",
        operation: "scan",
        stage: "advisory_evidence",
        protocol: "cli+artifact",
        method: "POST",
        route: "/api/providers/react-doctor/scan",
        command_template: "Invoke-EvidenceProvider.ps1 -Provider react-doctor -Operation scan -RepoPath <repo-path> -ArtifactDir <artifact-dir>",
        artifact: "react-doctor-native-result.json",
        required: false,
        status: "on_demand",
        source_spec: "npx --yes react-doctor@0.7.8 --json --no-telemetry",
        source_identity: Some("react-doctor 0.7.8 sha512-G3spmtZJE/gWWPRJ3rpgUWTPRDJpEmdRja7iNZ7RAXlfpEO+NWVzPTca/cPI9hLwPo2Aq5/BZggo5JDBrwGrlA=="),
        notes: "Pinned deterministic scanner; does not install skills or CI/configuration.",
    },
    ProviderOperation {
        provider: "react-doctor",
        operation: "adapt",
        stage: "advisory_evidence",
        protocol: "artifact+command",
        method: "POST",
        route: "/api/providers/react-doctor/adapt",
        command_template: "target/debug/code-intel.exe provider react-doctor-adapt --request <native.json|-> --artifact-root <artifact-dir> --evaluated-at <unix> --max-age-seconds <n>",
        artifact: "code-intel-evidence-route-result.v1",
        required: false,
        status: "active",
        source_spec: "Code Intel A04 advisory evidence admission over React Doctor JSON v3",
        source_identity: Some("react-doctor 0.7.8 sha512-G3spmtZJE/gWWPRJ3rpgUWTPRDJpEmdRja7iNZ7RAXlfpEO+NWVzPTca/cPI9hLwPo2Aq5/BZggo5JDBrwGrlA=="),
        notes: "Preserves deterministic diagnostic IDs, spans, tags, and per-project coverage.",
    },
];

pub fn run(options: &Options<'_>) -> Result<()> {
    let action = options.action.to_ascii_lowercase();
    let value = match action.as_str() {
        "list" => list(options.provider),
        "plan" => plan(options)?,
        "validate" => validate(),
        "invoke" => invoke(options)?,
        "compete-adapt" => adapt("compete", options)?,
        "react-doctor-adapt" => adapt("react-doctor", options)?,
        other => return Err(format!("unknown provider action: {other}").into()),
    };

    if options.json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        print_human(&value);
    }

    if action == "invoke" && value.get("ok").and_then(|value| value.as_bool()) == Some(false) {
        return Err("provider invoke failed".into());
    }

    Ok(())
}

fn adapt(provider: &str, options: &Options<'_>) -> Result<Value> {
    let request = options
        .request
        .ok_or("provider adapt requires --request <native.json|->")?;
    let artifact_root = options
        .artifact_root
        .ok_or("provider adapt requires --artifact-root <dir>")?;
    let evaluated_at = options
        .evaluated_at
        .ok_or("provider adapt requires --evaluated-at <unix>")?;
    let max_age_seconds = options
        .max_age_seconds
        .ok_or("provider adapt requires --max-age-seconds <n>")?;
    evidence::adapt(
        provider,
        request,
        artifact_root,
        evaluated_at,
        max_age_seconds,
    )
}

pub fn list(provider: Option<&str>) -> Value {
    let operations: Vec<Value> = OPERATIONS
        .iter()
        .filter(|operation| {
            provider
                .map(|value| value == operation.provider)
                .unwrap_or(true)
        })
        .map(operation_json)
        .collect();

    json!({
        "ok": true,
        "schema": "code-intel-provider-api.v1",
        "operations": operations
    })
}

pub fn plan(options: &Options<'_>) -> Result<Value> {
    let operation = find_required(options)?;
    let command = render_command(operation, options.repo);

    Ok(json!({
        "ok": true,
        "schema": "code-intel-provider-api.v1",
        "operation": operation_json(operation),
        "command": command
    }))
}

pub fn validate() -> Value {
    let mut seen = std::collections::HashSet::new();
    let mut errors = Vec::new();

    for operation in OPERATIONS {
        let key = format!("{}/{}", operation.provider, operation.operation);
        if !seen.insert(key.clone()) {
            errors.push(format!("duplicate provider operation: {key}"));
        }
        if !operation.route.starts_with('/') {
            errors.push(format!("{key} route must start with /"));
        }
        if operation.command_template.trim().is_empty() {
            errors.push(format!("{key} missing command template"));
        }
        if operation.artifact.trim().is_empty() {
            errors.push(format!("{key} missing artifact contract"));
        }
    }

    json!({
        "ok": errors.is_empty(),
        "schema": "code-intel-provider-api.v1",
        "operations": OPERATIONS.len(),
        "errors": errors
    })
}

pub fn invoke(options: &Options<'_>) -> Result<Value> {
    let operation = find_required(options)?;
    let repo_input = options
        .repo
        .ok_or("provider invoke requires --repo <path>")?;
    let repo = repo_input.canonicalize()?;

    match (operation.provider, operation.operation) {
        ("understand", "graph") => {
            invoke_understand(&repo, options.language, options.full, options.write)
        }
        ("understand", "graph_full") => invoke_understand(&repo, options.language, true, true),
        ("repowise", "status") => invoke_repowise(&repo, "status", &["--no-workspace"]),
        ("repowise", "index") => invoke_repowise_index(&repo),
        (provider, operation) => Err(format!(
            "provider invoke not implemented for {provider}/{operation}; use provider plan for compatibility command"
        )
        .into()),
    }
}

fn invoke_understand(repo: &Path, language: &str, full: bool, write: bool) -> Result<Value> {
    let graph = graph::generate(repo, language, full, write)?;
    Ok(json!({
        "ok": true,
        "schema": "code-intel-provider-api.v1",
        "provider": "understand",
        "operation": if full { "graph_full" } else { "graph" },
        "artifact": graph::graph_path(repo),
        "result": graph
    }))
}

fn invoke_repowise_index(repo: &Path) -> Result<Value> {
    let state_path = repo.join(".repowise").join("state.json");
    let db_path = repo.join(".repowise").join("wiki.db");
    if state_path.exists() || db_path.exists() {
        let update = invoke_repowise(repo, "update", &["--no-workspace", "--index-only"])?;
        let stderr = update
            .get("stderrTail")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if update.get("ok").and_then(|value| value.as_bool()) == Some(false)
            && stderr.contains("No previous sync found")
        {
            return invoke_repowise(
                repo,
                "init",
                &[
                    "--index-only",
                    "--no-agents",
                    "--no-codex",
                    "--no-distill-hook",
                ],
            );
        }
        Ok(update)
    } else {
        invoke_repowise(
            repo,
            "init",
            &[
                "--index-only",
                "--no-agents",
                "--no-codex",
                "--no-distill-hook",
            ],
        )
    }
}

fn invoke_repowise(repo: &Path, subcommand: &str, args: &[&str]) -> Result<Value> {
    let repo_cli = cli_path(repo);
    let mut child = Command::new("repowise")
        .arg(subcommand)
        .args(args)
        .arg(&repo_cli)
        .current_dir(&repo_cli)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(b"n\n")?;
    }

    let output = child.wait_with_output()?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let ok = output.status.success();

    Ok(json!({
        "ok": ok,
        "schema": "code-intel-provider-api.v1",
        "provider": "repowise",
        "operation": if subcommand == "update" || subcommand == "init" { "index" } else { subcommand },
        "exitCode": output.status.code().unwrap_or(-1),
        "artifact": repo_cli.join(".repowise").join("wiki.db"),
        "stdoutTail": tail(&stdout, 80),
        "stderrTail": tail(&stderr, 80)
    }))
}

fn cli_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(stripped) = text.strip_prefix(r"\\?\") {
        return PathBuf::from(stripped);
    }
    path.to_path_buf()
}

fn find_required<'a>(options: &Options<'a>) -> Result<&'static ProviderOperation> {
    let provider = options
        .provider
        .ok_or("provider action requires --provider")?;
    let operation = options
        .operation
        .ok_or("provider action requires --operation")?;
    find(provider, operation)
        .ok_or_else(|| format!("unknown provider operation: {provider}/{operation}").into())
}

pub fn find(provider: &str, operation: &str) -> Option<&'static ProviderOperation> {
    OPERATIONS
        .iter()
        .find(|item| item.provider == provider && item.operation == operation)
}

pub fn operation_json(operation: &ProviderOperation) -> Value {
    json!({
        "provider": operation.provider,
        "operation": operation.operation,
        "stage": operation.stage,
        "protocol": operation.protocol,
        "method": operation.method,
        "route": operation.route,
        "commandTemplate": operation.command_template,
        "artifact": operation.artifact,
        "required": operation.required,
        "status": operation.status,
        "sourceSpec": operation.source_spec,
        "sourceIdentity": operation.source_identity,
        "notes": operation.notes
    })
}

pub fn render_command(operation: &ProviderOperation, repo: Option<&Path>) -> String {
    match repo {
        Some(repo) => operation
            .command_template
            .replace("<repo-path>", &repo.to_string_lossy()),
        None => operation.command_template.to_string(),
    }
}

fn print_human(value: &Value) {
    if let Some(operations) = value.get("operations").and_then(|value| value.as_array()) {
        for operation in operations {
            println!(
                "{}:{} {} {} -> {}",
                operation["provider"].as_str().unwrap_or(""),
                operation["operation"].as_str().unwrap_or(""),
                operation["method"].as_str().unwrap_or(""),
                operation["route"].as_str().unwrap_or(""),
                operation["commandTemplate"].as_str().unwrap_or("")
            );
        }
        return;
    }
    println!("{value}");
}

fn tail(value: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = value.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_registry_validates() {
        let result = validate();
        assert_eq!(result["ok"].as_bool(), Some(true));
        assert!(result["operations"].as_u64().unwrap_or(0) >= 6);
    }

    #[test]
    fn repowise_and_understand_share_schema() {
        let repowise = operation_json(find("repowise", "index").unwrap());
        let understand = operation_json(find("understand", "graph").unwrap());

        for key in [
            "provider",
            "operation",
            "protocol",
            "route",
            "commandTemplate",
            "artifact",
            "sourceSpec",
        ] {
            assert!(repowise.get(key).is_some(), "repowise missing {key}");
            assert!(understand.get(key).is_some(), "understand missing {key}");
        }
    }

    #[test]
    fn on_demand_evidence_providers_are_public_and_optional() {
        for (provider, operations) in [
            ("compete", ["prepare", "status", "adapt"].as_slice()),
            ("react-doctor", ["scan", "adapt"].as_slice()),
        ] {
            for operation in operations {
                let item = find(provider, operation).expect("provider operation should exist");
                assert!(!item.required);
                assert_eq!(item.stage, "advisory_evidence");
                assert!(item.source_identity.is_some());
            }
        }
        assert!(find("react-doctor", "scan")
            .unwrap()
            .command_template
            .contains("Invoke-EvidenceProvider.ps1"));
        assert!(find("react-doctor", "adapt")
            .unwrap()
            .command_template
            .contains("react-doctor-adapt"));
    }
}
