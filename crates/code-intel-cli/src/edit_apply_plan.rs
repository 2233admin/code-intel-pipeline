//! `code-intel edit apply-plan` — the agent-facing front door that turns a
//! plan into an edit (#96 item 2, charter gate G4 in #139).
//!
//! The economy this route exists for: `capability exec edit.ast-grep-plan`
//! produces the plan, and this command applies it by *path*. The model never
//! reads the matched bytes, never restates a line, and never sees a
//! replacement it did not already ask ast-grep to compute. What it spends is
//! one pattern and one rewrite, no matter how many call sites move.
//!
//! Like `edit apply`, this module owns argument shape and rendering and
//! nothing else. The write goes through the `edit.ast-grep-apply` capability
//! envelope, with `implementation` and `allowedEffects` read from the
//! registered declaration through `ExecutionPolicy`. No repository byte is
//! written in this file.
//!
//! The snapshot is built fresh, over exactly the files the plan names —
//! deliberately not pinned to the plan's own snapshot. Pinning would refuse a
//! perfectly good plan because some unrelated file in the scanned scope
//! changed, and would move the refusal off the per-span digests, which are
//! the only guard that can say *which match* drifted.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::edit_apply::{refuse, Staging};
use crate::{capability, capability_inventory, execution_policy, snapshot};

const CAPABILITY: &str = "edit.ast-grep-apply";
const USAGE: &str = "usage: edit apply-plan --repo-path <checkout> --plan <structured-edit-plan.json> [--out <staging-dir>] [--manifest <integrations.json>] [--envelope]";

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let cli = match Cli::parse(raw) {
        Ok(cli) => cli,
        Err(message) => return refuse(CAPABILITY, 64, "arguments", &message),
    };
    match execute(&cli) {
        Ok((exit_code, document)) => {
            println!(
                "{}",
                serde_json::to_string(&document).expect("edit apply-plan result serializes")
            );
            exit_code
        }
        // Same rule as `edit apply`: an exit the route contract declares must
        // not be answered with zero stdout bytes. `refuse` publishes the
        // shared `code-intel-edit-failure.v1` document and keeps the
        // actionable sentence on stderr.
        Err((exit_code, stage, message)) => refuse(CAPABILITY, exit_code, stage, &message),
    }
}

struct Cli {
    repo_path: PathBuf,
    plan: PathBuf,
    out: Option<PathBuf>,
    manifest: Option<PathBuf>,
    envelope: bool,
}

impl Cli {
    fn parse(raw: &[String]) -> Result<Self, String> {
        let mut repo_path: Option<PathBuf> = None;
        let mut plan: Option<PathBuf> = None;
        let mut out: Option<PathBuf> = None;
        let mut manifest: Option<PathBuf> = None;
        let mut envelope = false;
        let mut index = 0;
        while index < raw.len() {
            let flag = raw[index].as_str();
            if flag == "--envelope" {
                if envelope {
                    return Err("duplicate --envelope".into());
                }
                envelope = true;
                index += 1;
                continue;
            }
            let value = raw
                .get(index + 1)
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| format!("{flag} requires exactly one value"))?;
            let slot = match flag {
                "--repo-path" => &mut repo_path,
                "--plan" => &mut plan,
                "--out" => &mut out,
                "--manifest" => &mut manifest,
                other => return Err(format!("unknown argument for edit apply-plan: {other}")),
            };
            if slot.replace(PathBuf::from(value)).is_some() {
                return Err(format!("duplicate {flag}"));
            }
            index += 2;
        }
        Ok(Self {
            repo_path: repo_path.ok_or_else(|| USAGE.to_string())?,
            plan: plan.ok_or_else(|| USAGE.to_string())?,
            out,
            manifest,
            envelope,
        })
    }
}

fn execute(cli: &Cli) -> Result<(i32, Value), (i32, &'static str, String)> {
    let repo = fs::canonicalize(&cli.repo_path).map_err(|error| {
        (
            65,
            "repository",
            format!(
                "cannot resolve --repo-path {}: {error}",
                cli.repo_path.display()
            ),
        )
    })?;
    let bytes = fs::read(&cli.plan).map_err(|error| {
        (
            74,
            "plan",
            format!("cannot read --plan {}: {error}", cli.plan.display()),
        )
    })?;
    let plan: Value = serde_json::from_slice(&bytes).map_err(|error| {
        (
            64,
            "plan",
            format!("--plan is not one JSON document: {error}"),
        )
    })?;
    let plan_sha256 = capability::sha256_hex(&bytes);
    let scope = plan_scope(&plan)?;
    // Scope the snapshot to exactly the files the plan touches. A
    // repository-wide scope would make a two-site rename pay for a full-tree
    // digest, which is how a correct primitive becomes one nobody calls.
    let snapshot = snapshot::build_for_dag(&repo, "explicit_overlay", &scope).map_err(|error| {
        (
            65,
            "snapshot",
            format!("snapshot identity for the planned files: {error}"),
        )
    })?;
    let declaration = capability::declaration_for(CAPABILITY, cli.manifest.as_deref())
        .map_err(|error| (69, "registry", error))?;
    let policy =
        execution_policy::ExecutionPolicy::for_profile(execution_policy::RunProfile::Default);
    let request = json!({
        "schema": "code-intel-capability-request.v1",
        "capability": CAPABILITY,
        "contractVersion": 1,
        "implementation": declaration["implementation"],
        "snapshot": snapshot["snapshot"],
        "options": {"repoPath": repo, "plan": plan},
        "inputs": [],
        "effectPolicy": {"allowedEffects": policy.allowed_effects(&declaration)},
    });

    let staging = Staging::open(cli.out.clone()).map_err(|error| (74, "staging", error))?;
    let outcome = capability::exec_in_process(
        CAPABILITY,
        &request,
        staging.path(),
        cli.manifest.as_deref(),
        capability_inventory::execute,
    );
    let result = outcome
        .result
        .as_ref()
        .and_then(|result| result["artifacts"][0]["path"].as_str())
        .and_then(|relative| fs::read(staging.path().join(relative)).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    Ok((outcome.exit_code, render(cli, outcome, result, plan_sha256)))
}

/// The files the plan names, deduplicated — also the proof that a plan with
/// no applicable edits never reaches the capability.
fn plan_scope(plan: &Value) -> Result<Vec<String>, (i32, &'static str, String)> {
    let edits = plan["apply"]["edits"].as_array().ok_or_else(|| {
        (
            64,
            "plan",
            "--plan carries no apply.edits; it does not have the shape edit.ast-grep-plan publishes"
                .to_string(),
        )
    })?;
    let scope = edits
        .iter()
        .filter_map(|edit| edit["file"].as_str())
        .map(|file| file.replace('\\', "/"))
        .collect::<BTreeSet<_>>();
    if scope.is_empty() {
        return Err((
            64,
            "plan",
            format!(
                "--plan has no applicable edits: {}",
                plan["apply"]["reason"]
                    .as_str()
                    .unwrap_or("no reason recorded")
            ),
        ));
    }
    Ok(scope.into_iter().collect())
}

/// Answer with the evidence and nothing else. A route whose purpose is to
/// make a mechanical change cheap must not spend the saving on its own
/// reply: the per-edit rows and the capability envelope stay in the artifact
/// unless `--envelope` asks for them.
fn render(
    cli: &Cli,
    outcome: capability::ExecOutcome,
    result: Option<Value>,
    plan_sha256: String,
) -> Value {
    let refused = result
        .as_ref()
        .map(|value| {
            value["edits"].as_array().map_or(Vec::new(), |rows| {
                rows.iter()
                    .filter(|row| row["verdict"] != "match")
                    .cloned()
                    .collect::<Vec<_>>()
            })
        })
        .unwrap_or_default();
    json!({
        "schema": "code-intel-edit-apply-plan.v1",
        "capability": CAPABILITY,
        "exitCode": outcome.exit_code,
        "applied": result
            .as_ref()
            .and_then(|value| value["applied"].as_bool())
            .unwrap_or(false),
        "planSha256": plan_sha256,
        "summary": result.as_ref().map(|value| value["summary"].clone()),
        "files": result.as_ref().map(|value| value["files"].clone()),
        "drifted": refused,
        "refusal": result.as_ref().map(|value| value["refusal"].clone()),
        "envelope": if cli.envelope { outcome.result } else { None },
        "applyResult": if cli.envelope { result } else { None },
        "diagnostic": outcome.diagnostic,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn the_route_takes_a_plan_path_and_never_replacement_bytes() {
        let cli = Cli::parse(&args(&[
            "--repo-path",
            ".",
            "--plan",
            "plan.json",
            "--envelope",
        ]))
        .expect("plan route parses");
        assert_eq!(cli.plan, PathBuf::from("plan.json"));
        assert!(cli.envelope);
        // There is no flag through which a caller could supply file contents,
        // a span, or a replacement: the plan is the only instruction.
        assert!(Cli::parse(&args(&[
            "--repo-path",
            ".",
            "--plan",
            "plan.json",
            "--replacement",
            "x"
        ]))
        .is_err());
        assert!(Cli::parse(&args(&["--repo-path", "."])).is_err());
    }

    #[test]
    fn a_plan_with_no_applicable_edits_never_reaches_the_capability() {
        let plan = json!({
            "apply": {"applicable": false, "reason": "plan matched nothing", "edits": []}
        });
        let (exit_code, stage, message) = plan_scope(&plan).unwrap_err();
        assert_eq!(exit_code, 64);
        assert_eq!(stage, "plan");
        assert!(message.contains("plan matched nothing"), "{message}");
    }

    #[test]
    fn the_snapshot_scope_is_exactly_the_files_the_plan_names() {
        let plan = json!({
            "apply": {"edits": [
                {"file": "src/b.rs"},
                {"file": "src/a.rs"},
                {"file": "src/b.rs"}
            ]}
        });
        assert_eq!(
            plan_scope(&plan).unwrap(),
            vec!["src/a.rs".to_string(), "src/b.rs".to_string()]
        );
    }
}
