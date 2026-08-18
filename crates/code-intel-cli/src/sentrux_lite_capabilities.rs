use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};

pub(super) fn provider_discovery_json() -> Result<Value, String> {
    Ok(json!({
        "provider":"sentrux",
        "mode":"builtin_lite",
        "available":true,
        "operations":["scan","health","dsm","git_stats","evolution","test_gaps","what_if","check_rules","check","gate","rescan"],
        "aliases":["pro_status","plugin_list","plugin_validate"],
        "explicitAuthorityOperations":["gate_save"],
        "lifecycleOperations":["session_start","session_end"],
        "legacyFallback":"legacy/Invoke-SentruxAgentTool.ps1"
    }))
}

fn git_command(repo: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|error| format!("start git {}: {error}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| format!("git output is not UTF-8: {error}"))
}

pub(super) fn git_stats_json(repo: &Path) -> Result<Value, String> {
    let count_output = match git_command(repo, &["rev-list", "--count", "HEAD"]) {
        Ok(output) => output,
        Err(reason) => {
            return Ok(json!({
                "commitCount":0,
                "recentCommits":[],
                "status":"unavailable",
                "reason":reason
            }))
        }
    };
    let count = count_output
        .trim()
        .parse::<u64>()
        .map_err(|error| format!("git commit count is invalid: {error}"))?;
    let recent_output = match git_command(repo, &["log", "-n", "20", "--format=%H%x09%aI"]) {
        Ok(output) => output,
        Err(reason) => {
            return Ok(json!({
                "commitCount":count,
                "recentCommits":[],
                "status":"unavailable",
                "reason":reason
            }))
        }
    };
    let recent = recent_output
        .lines()
        .filter_map(|line| {
            let (commit, authored_at) = line.split_once('\t')?;
            Some(json!({"commit":commit,"authoredAt":authored_at}))
        })
        .collect::<Vec<_>>();
    Ok(json!({"commitCount":count,"recentCommits":recent,"status":"ok"}))
}

pub(super) fn evolution_json(repo: &Path) -> Result<Value, String> {
    let recent_output = match git_command(repo, &["log", "-n", "20", "--format=%H%x09%aI%x09%an"]) {
        Ok(output) => output,
        Err(reason) => {
            return Ok(json!({
                "status":"unavailable",
                "windowCommits":0,
                "trend":"unknown",
                "recentCommits":[],
                "reason":reason
            }))
        }
    };
    let recent = recent_output
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            Some(json!({
                "commit":fields.next()?,
                "authoredAt":fields.next()?,
                "author":fields.next()?
            }))
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "status":"ok",
        "windowCommits":recent.len(),
        "trend":"observed",
        "recentCommits":recent
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn history_capabilities_are_auditable_without_git_history() {
        let repo = std::path::Path::new("this-path-does-not-contain-a-git-checkout");
        let stats =
            super::git_stats_json(repo).expect("missing git history is a valid lite result");
        let evolution =
            super::evolution_json(repo).expect("missing git history is a valid lite result");

        assert_eq!(stats["status"], "unavailable");
        assert_eq!(stats["commitCount"], 0);
        assert_eq!(evolution["status"], "unavailable");
        assert_eq!(evolution["windowCommits"], 0);
        assert_eq!(evolution["trend"], "unknown");
    }

    #[test]
    fn what_if_is_a_bounded_snapshot_capability() {
        let root = std::env::temp_dir().join(format!(
            "code-intel-lite-what-if-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join(".sentrux")).expect("rules directory");
        std::fs::create_dir_all(root.join("src")).expect("source directory");
        std::fs::write(
            root.join(".sentrux/rules.toml"),
            "max_cc = 1\nmax_coupling = 1\n",
        )
        .expect("rules");
        std::fs::write(
            root.join("src/lib.rs"),
            "pub fn sample() { if true { println!(\"x\"); } }\n",
        )
        .expect("source");

        let value = super::what_if_json(&root).expect("what_if should be available");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["summary"]["scenarioCount"], 4);
        assert!(value["summary"]["failingScenarioCount"].as_u64().unwrap() > 0);
        assert_eq!(value["scenarios"].as_array().unwrap().len(), 4);
        assert!(value["limitations"].as_array().unwrap().len() >= 2);
        let _ = std::fs::remove_dir_all(root);
    }
}

pub(super) fn test_gaps_json(repo: &Path) -> Result<Value, String> {
    let mut source_files = 0_u64;
    let mut test_files = 0_u64;
    let mut stack = vec![repo.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory).map_err(|error| {
            format!(
                "read test inventory directory {}: {error}",
                directory.display()
            )
        })? {
            let entry = entry.map_err(|error| format!("read test inventory entry: {error}"))?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if entry
                .file_type()
                .map_err(|error| format!("inspect test inventory entry: {error}"))?
                .is_dir()
            {
                if !matches!(name.as_str(), ".git" | "target" | "node_modules") {
                    stack.push(path);
                }
                continue;
            }
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if !matches!(
                extension,
                "rs" | "py" | "js" | "ts" | "tsx" | "go" | "java" | "cs"
            ) {
                continue;
            }
            let relative = path
                .strip_prefix(repo)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_ascii_lowercase();
            if relative.contains("test") || relative.contains("spec") {
                test_files += 1;
            } else {
                source_files += 1;
            }
        }
    }
    Ok(json!({
        "status":"heuristic",
        "sourceFiles":source_files,
        "testFiles":test_files,
        "gapStatus":if test_files == 0 { "unknown" } else { "inventory_only" },
        "limitations":["This lite fallback inventories test files; it does not prove symbol-level test coverage."]
    }))
}

pub(super) fn what_if_json(repo: &Path) -> Result<Value, String> {
    let dsm = super::sentrux_analysis::analyze(repo)?;
    let max_cc = read_rule_number(repo, "max_cc").unwrap_or(25.0);
    let max_coupling = read_rule_number(repo, "max_coupling").unwrap_or(76.0);
    let blast_limit = max_coupling + 2.0;

    let complexity = dsm["file_details"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|file| {
            file["functions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|function| {
                    let value = function["complexity"].as_f64()?;
                    (value > max_cc).then(|| {
                        json!({
                            "id":function["id"],
                            "name":function["name"],
                            "file":file["path"],
                            "sourceAnchor":function["source_anchor"],
                            "value":value,
                            "limit":max_cc,
                            "overBy":value - max_cc
                        })
                    })
                })
        })
        .collect::<Vec<_>>();
    let coupling = dsm["modules"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|module| {
            let value = module["metrics"]["coupling"].as_f64()?;
            (value > max_coupling).then(|| {
                json!({
                    "id":module["id"],
                    "name":module["name"],
                    "metric":"coupling",
                    "value":value,
                    "limit":max_coupling,
                    "risk":module["metrics"]["risk"]
                })
            })
        })
        .collect::<Vec<_>>();
    let blast_radius = dsm["modules"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|module| {
            let value = module["metrics"]["blast_radius"].as_f64()?;
            (value > blast_limit).then(|| {
                json!({
                    "id":module["id"],
                    "name":module["name"],
                    "metric":"blast_radius",
                    "value":value,
                    "limit":blast_limit,
                    "risk":module["metrics"]["risk"]
                })
            })
        })
        .collect::<Vec<_>>();
    let test_gaps = dsm["modules"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|module| {
            let value = module["metrics"]["test_gap"].as_f64()?;
            (value > 0.0).then(|| {
                json!({
                    "id":module["id"],
                    "name":module["name"],
                    "metric":"test_gap",
                    "value":value,
                    "limit":0,
                    "risk":module["metrics"]["risk"]
                })
            })
        })
        .collect::<Vec<_>>();

    let scenarios = vec![
        what_if_scenario(
            "current_max_cc_gate",
            "max_cc",
            max_cc,
            complexity,
            "Split or simplify functions above the current Sentrux complexity ceiling.",
        ),
        what_if_scenario(
            "module_coupling_cap",
            "max_coupling",
            max_coupling,
            coupling,
            "Inspect dependency edges and preserve provider boundaries before adding coupling.",
        ),
        what_if_scenario(
            "blast_radius_cap",
            "max_blast_radius",
            blast_limit,
            blast_radius,
            "Reduce fan-out or split the highest-impact module before expanding its surface.",
        ),
        what_if_scenario(
            "test_gap_gate",
            "test_gap",
            0.0,
            test_gaps,
            "Add or select tests for source-heavy modules before treating the change as fully covered.",
        ),
    ];
    let failing = scenarios
        .iter()
        .filter(|scenario| scenario["pass"] == false)
        .count();
    let primary_risk = scenarios
        .iter()
        .find(|scenario| scenario["pass"] == false)
        .and_then(|scenario| scenario["id"].as_str())
        .unwrap_or("none");
    Ok(json!({
        "status":"ok",
        "scope":"repository_snapshot",
        "rules":{
            "max_cc":max_cc,
            "max_coupling":max_coupling,
            "max_blast_radius":blast_limit,
            "source":if repo.join(".sentrux/rules.toml").is_file() { "repository" } else { "defaults" }
        },
        "scenarios":scenarios,
        "summary":{
            "scenarioCount":4,
            "failingScenarioCount":failing,
            "primaryRisk":primary_risk
        },
        "limitations":[
            "Lite what_if evaluates the current snapshot; it does not mutate or synthesize a hypothetical checkout.",
            "Function and dependency extraction are heuristic and remain bounded by the lite DSM parser."
        ]
    }))
}

fn read_rule_number(repo: &Path, name: &str) -> Option<f64> {
    let text = fs::read_to_string(repo.join(".sentrux/rules.toml")).ok()?;
    text.lines()
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == name).then(|| value.trim().trim_matches('"').parse().ok())
        })
        .flatten()
}

fn what_if_scenario(
    id: &str,
    metric: &str,
    limit: f64,
    affected: Vec<Value>,
    action: &str,
) -> Value {
    let pass = affected.is_empty();
    json!({
        "id":id,
        "metric":metric,
        "pass":pass,
        "severity":if pass { "ok" } else { "high" },
        "impactCount":affected.len(),
        "affected":affected.into_iter().take(20).collect::<Vec<_>>(),
        "limit":limit,
        "action":action
    })
}
