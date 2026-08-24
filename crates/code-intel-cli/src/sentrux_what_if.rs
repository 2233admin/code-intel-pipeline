//! `what_if` half of the Sentrux evolution/what-if Rust port — see the
//! module doc on `sentrux_evolution.rs` (the parent module this file is
//! included into) for why this is split out.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::sentrux_analysis;

use super::{as_array, evolution_bus_factor};

struct RuleHints {
    exists: bool,
    path: PathBuf,
    max_cycles: Option<i64>,
    max_cc: Option<i64>,
    max_coupling: Option<String>,
    no_god_files: Option<bool>,
    ignore_test_dependencies: bool,
    pollution_exclusions: Vec<String>,
}

fn rule_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let (candidate_key, rest) = line.split_once('=')?;
        if candidate_key.trim() != key {
            return None;
        }
        let value = rest.split('#').next().unwrap_or(rest).trim();
        Some(value.to_string())
    })
}

fn parse_toml_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_pollution_list(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    let items: Vec<&str> = if let Some(inner) = trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    {
        inner.split(',').collect()
    } else {
        vec![trimmed]
    };
    let mut normalized: Vec<String> = items
        .into_iter()
        .filter_map(|item| {
            let item = item.trim().trim_matches('\'').trim_matches('"');
            let item = item
                .trim_start_matches(['\\', '/'])
                .trim_end_matches(['\\', '/']);
            if item.is_empty() {
                None
            } else {
                Some(
                    item.to_ascii_lowercase()
                        .replace('/', std::path::MAIN_SEPARATOR_STR),
                )
            }
        })
        .collect();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn read_rule_hints(repo: &Path) -> RuleHints {
    let path = repo.join(".sentrux").join("rules.toml");
    if !path.is_file() {
        return RuleHints {
            exists: false,
            path,
            max_cycles: None,
            max_cc: None,
            max_coupling: None,
            no_god_files: None,
            ignore_test_dependencies: false,
            pollution_exclusions: Vec::new(),
        };
    }
    let text = fs::read_to_string(&path).unwrap_or_default();
    RuleHints {
        exists: true,
        max_cycles: rule_value(&text, "max_cycles").and_then(|v| v.parse::<i64>().ok()),
        max_cc: rule_value(&text, "max_cc").and_then(|v| v.parse::<i64>().ok()),
        max_coupling: rule_value(&text, "max_coupling")
            .map(|v| v.trim_matches('"').trim_matches('\'').to_string()),
        no_god_files: rule_value(&text, "no_god_files").and_then(|v| parse_toml_bool(&v)),
        ignore_test_dependencies: rule_value(&text, "ignore_test_dependencies")
            .and_then(|v| parse_toml_bool(&v))
            .unwrap_or(false),
        pollution_exclusions: rule_value(&text, "pollution_exclusions")
            .map(|v| parse_pollution_list(&v))
            .unwrap_or_default(),
        path,
    }
}

fn coupling_grade_to_limit(grade: Option<&str>) -> i64 {
    match grade.map(|value| value.trim().to_ascii_uppercase()) {
        None => 5,
        Some(value) => match value.as_str() {
            "A" => 2,
            "B" => 5,
            "C" => 8,
            "D" => 12,
            other => other.parse::<i64>().unwrap_or(5),
        },
    }
}

struct NoisyDir {
    path: &'static str,
    reason: &'static str,
    inspect_nested_git: bool,
}

const NOISY_DIRS: &[NoisyDir] = &[
    NoisyDir {
        path: "node_modules",
        reason: "dependency directory excluded from governed source graph",
        inspect_nested_git: false,
    },
    NoisyDir {
        path: ".pnpm",
        reason: "dependency store excluded from governed source graph",
        inspect_nested_git: false,
    },
    NoisyDir {
        path: ".yarn",
        reason: "dependency store excluded from governed source graph",
        inspect_nested_git: false,
    },
    NoisyDir {
        path: "vendor",
        reason: "vendored code excluded from governed source graph",
        inspect_nested_git: true,
    },
    NoisyDir {
        path: "vendors",
        reason: "vendored code excluded from governed source graph",
        inspect_nested_git: true,
    },
    NoisyDir {
        path: "third_party",
        reason: "third-party code excluded from governed source graph",
        inspect_nested_git: true,
    },
    NoisyDir {
        path: "third-party",
        reason: "third-party code excluded from governed source graph",
        inspect_nested_git: true,
    },
    NoisyDir {
        path: "external",
        reason: "external code excluded from governed source graph",
        inspect_nested_git: true,
    },
    NoisyDir {
        path: "research",
        reason: "research or reference tree excluded from governed source graph",
        inspect_nested_git: true,
    },
    NoisyDir {
        path: "sandbox",
        reason: "sandbox tree excluded from governed source graph",
        inspect_nested_git: false,
    },
    NoisyDir {
        path: "dist",
        reason: "build output excluded from governed source graph",
        inspect_nested_git: false,
    },
    NoisyDir {
        path: "build",
        reason: "build output excluded from governed source graph",
        inspect_nested_git: false,
    },
    NoisyDir {
        path: "target",
        reason: "build output excluded from governed source graph",
        inspect_nested_git: false,
    },
    NoisyDir {
        path: "static/assets",
        reason: "bundled static assets excluded from governed source graph",
        inspect_nested_git: false,
    },
    NoisyDir {
        path: "public/assets",
        reason: "bundled static assets excluded from governed source graph",
        inspect_nested_git: false,
    },
    NoisyDir {
        path: "tools",
        reason: "common tool or generated-support directory",
        inspect_nested_git: true,
    },
];

fn count_nested_git_dirs(root: &Path, limit: usize) -> usize {
    let mut count = 0;
    let mut stack = vec![root.to_path_buf()];
    'walk: while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if path.file_name().and_then(|name| name.to_str()) == Some(".git") {
                count += 1;
                if count >= limit {
                    break 'walk;
                }
                continue;
            }
            stack.push(path);
        }
    }
    count.min(limit)
}

fn pollution_signals(repo: &Path, ignored: &[String]) -> Vec<Value> {
    let ignored_set: HashSet<String> = ignored
        .iter()
        .filter(|entry| !entry.trim().is_empty())
        .map(|entry| {
            entry
                .trim()
                .trim_matches(['\\', '/'])
                .to_ascii_lowercase()
                .replace('/', std::path::MAIN_SEPARATOR_STR)
        })
        .collect();

    let mut signals = Vec::new();
    for entry in NOISY_DIRS {
        let normalized_dir = entry
            .path
            .to_ascii_lowercase()
            .replace('/', std::path::MAIN_SEPARATOR_STR);
        if ignored_set.contains(&normalized_dir) {
            continue;
        }
        let full = repo.join(entry.path);
        if !full.is_dir() {
            continue;
        }
        let nested_git_count = if entry.inspect_nested_git {
            count_nested_git_dirs(&full, 5)
        } else {
            0
        };
        signals.push(json!({
            "path": entry.path,
            "nested_git_count_sample": nested_git_count,
            "reason": if nested_git_count > 0 { "contains nested repositories".to_string() } else { entry.reason.to_string() },
        }));
    }
    signals
}

fn scope_candidates(repo: &Path) -> Vec<Value> {
    let mut found: Vec<PathBuf> = Vec::new();
    let mut stack = vec![repo.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if found.len() >= 12 {
            break;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.file_name().and_then(|name| name.to_str()) != Some("baseline.json") {
                continue;
            }
            let is_sentrux_baseline = path
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                == Some(".sentrux");
            if !is_sentrux_baseline {
                continue;
            }
            found.push(path);
            if found.len() >= 12 {
                break;
            }
        }
    }
    found
        .into_iter()
        .map(|baseline_path| {
            let scope = baseline_path
                .parent()
                .and_then(|parent| parent.parent())
                .map(Path::to_path_buf)
                .unwrap_or_else(|| repo.to_path_buf());
            json!({
                "path": scope.display().to_string(),
                "relative_path": relative_path_safe(repo, &scope),
                "baseline": baseline_path.display().to_string(),
            })
        })
        .collect()
}

fn relative_path_safe(base: &Path, target: &Path) -> String {
    target
        .strip_prefix(base)
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| target.display().to_string())
}

fn what_if_function_violations(dsm: &Value, max_complexity: i64) -> Vec<Value> {
    let mut scored: Vec<(i64, Value)> = Vec::new();
    for file in as_array(dsm, "file_details") {
        for function in as_array(&file, "functions") {
            let complexity = function["complexity"].as_i64().unwrap_or(0);
            if complexity <= max_complexity {
                continue;
            }
            let over_by = complexity - max_complexity;
            scored.push((
                over_by,
                json!({
                    "id": function["id"], "name": function["name"], "file": file["path"],
                    "sourceAnchor": function["source_anchor"], "complexity": complexity,
                    "limit": max_complexity, "over_by": over_by,
                }),
            ));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, value)| value).collect()
}

fn what_if_god_file_violations(dsm: &Value, max_loc: i64, max_functions: i64) -> Vec<Value> {
    let mut scored: Vec<(i64, Value)> = as_array(dsm, "file_details")
        .into_iter()
        .filter_map(|file| {
            let loc = file["loc"].as_i64().unwrap_or(0);
            let functions = file["function_count"].as_i64().unwrap_or(0);
            if loc <= max_loc && functions <= max_functions {
                return None;
            }
            let over = (loc - max_loc).max(functions - max_functions);
            Some((
                over,
                json!({
                    "id": file["id"], "path": file["path"], "sourceAnchor": file["source_anchor"],
                    "loc": loc, "functionCount": functions, "maxLoc": max_loc, "maxFunctions": max_functions,
                }),
            ))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, value)| value).collect()
}

fn what_if_module_metric_violations(dsm: &Value, metric: &str, limit: i64) -> Vec<Value> {
    let mut scored: Vec<(i64, Value)> = as_array(dsm, "modules")
        .into_iter()
        .filter_map(|module| {
            let value = module["metrics"][metric].as_i64().unwrap_or(0);
            if value <= limit {
                return None;
            }
            Some((
                value,
                json!({
                    "id": module["id"], "name": module["name"], "metric": metric, "value": value, "limit": limit,
                    "files": module["files"], "risk": module["metrics"]["risk"], "coupling": module["metrics"]["coupling"],
                    "blastRadius": module["metrics"]["blast_radius"], "testGap": module["metrics"]["test_gap"],
                }),
            ))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, value)| value).collect()
}

#[allow(clippy::too_many_arguments)]
fn what_if_scenario(
    name: &str,
    question: &str,
    pass: bool,
    affected: &[Value],
    severity: &str,
    recommended_rule: &str,
    action: &str,
) -> Value {
    json!({
        "name": name,
        "question": question,
        "pass": pass,
        "severity": severity,
        "impact_count": affected.len(),
        "affected": affected.iter().take(20).cloned().collect::<Vec<_>>(),
        "recommended_rule": recommended_rule,
        "action": action,
    })
}

fn iso8601_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let secs = (nanos / 1_000_000_000) as i64;
    let millis = (nanos / 1_000_000 % 1_000) as i64;
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn what_if(repo: &Path) -> Result<Value, String> {
    let rule_hints = read_rule_hints(repo);
    let dsm = sentrux_analysis::analyze(repo)?;

    let max_cc = rule_hints.max_cc.unwrap_or(25);
    let strict_cc = max_cc.min(15);
    let hard_cc = strict_cc.min(10);
    let coupling_limit = coupling_grade_to_limit(rule_hints.max_coupling.as_deref());
    let blast_limit = (coupling_limit + 2).max(3);
    let god_loc_limit = 800;
    let god_function_limit = 40;
    let bus_risk_limit = 85;

    let complexity_at_rule = what_if_function_violations(&dsm, max_cc);
    let complexity_strict = what_if_function_violations(&dsm, strict_cc);
    let complexity_hard = what_if_function_violations(&dsm, hard_cc);
    let god_files_violations = what_if_god_file_violations(&dsm, god_loc_limit, god_function_limit);
    let coupling = what_if_module_metric_violations(&dsm, "coupling", coupling_limit);
    let blast = what_if_module_metric_violations(&dsm, "blast_radius", blast_limit);
    let test_gaps = what_if_module_metric_violations(&dsm, "test_gap", 0);
    let bus_factor = evolution_bus_factor(repo, &dsm);
    let bus_factor_risk_list: Vec<Value> = bus_factor["modules"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|module| {
            module["bus_factor_risk"].as_i64().unwrap_or(0) >= bus_risk_limit
                && module["touches"].as_i64().unwrap_or(0) > 0
        })
        .take(20)
        .map(|module| {
            json!({
                "id": module["id"], "name": module["name"], "value": module["bus_factor_risk"],
                "risk": module["bus_factor_risk"], "files": module["files"], "bus_factor": module["bus_factor"],
                "touches": module["touches"], "top_author": module["top_author"], "top_author_share": module["top_author_share"],
            })
        })
        .collect();

    // god_files_violations is computed to mirror the PS1 tool's full metric
    // set, matching `$godFiles` there — the PS1 `Invoke-WhatIfTool` also
    // computes it but never turns it into its own scenario; keep parity by
    // leaving it unused as a scenario input here too.
    let _ = &god_files_violations;

    let mut pollution_exclusions = rule_hints.pollution_exclusions.clone();
    pollution_exclusions.sort();
    pollution_exclusions.dedup();
    let pollution = pollution_signals(repo, &pollution_exclusions);
    let scope_candidates_value = scope_candidates(repo);

    let scenarios = vec![
        what_if_scenario(
            "current_max_cc_gate",
            "Would the current or default max_cc gate pass?",
            complexity_at_rule.is_empty(),
            &complexity_at_rule,
            if complexity_at_rule.is_empty() { "ok" } else { "high" },
            &format!("max_cc = {max_cc}"),
            "Split or simplify functions above the max_cc gate before raising the baseline.",
        ),
        what_if_scenario(
            "strict_max_cc_gate",
            "What breaks if max_cc is tightened for agent-written code?",
            complexity_strict.is_empty(),
            &complexity_strict,
            if complexity_strict.is_empty() { "ok" } else { "medium" },
            &format!("max_cc = {strict_cc}"),
            "Use this as the target for touched/new code; do not require legacy cleanup in one pass.",
        ),
        what_if_scenario(
            "hard_max_cc_gate",
            "What breaks under a very strict complexity ceiling?",
            complexity_hard.is_empty(),
            &complexity_hard,
            if complexity_hard.is_empty() { "ok" } else { "medium" },
            &format!("max_cc = {hard_cc}"),
            "Use only for greenfield modules or narrow critical paths.",
        ),
        what_if_scenario(
            "module_coupling_cap",
            "Which modules would fail a coupling cap?",
            coupling.is_empty(),
            &coupling,
            if coupling.is_empty() { "ok" } else { "high" },
            "max_coupling = \"B\"",
            "Inspect top edges and carve adapters before adding new cross-module dependencies.",
        ),
        what_if_scenario(
            "blast_radius_cap",
            "Which modules would fail a blast-radius cap?",
            blast.is_empty(),
            &blast,
            if blast.is_empty() { "ok" } else { "high" },
            &format!("max_blast_radius = {blast_limit}"),
            "Reduce incident dependencies or split fan-out responsibilities.",
        ),
        what_if_scenario(
            "test_gap_gate",
            "Which modules would fail if every source-heavy module needed tests?",
            test_gaps.is_empty(),
            &test_gaps,
            if test_gaps.is_empty() { "ok" } else { "medium" },
            "require_tests_for_source_modules = true",
            "Add targeted smoke or contract tests around the highest-risk untested modules.",
        ),
        what_if_scenario(
            "bus_factor_gate",
            "Which modules are one-person or no-history risks?",
            bus_factor_risk_list.is_empty(),
            &bus_factor_risk_list,
            if bus_factor_risk_list.is_empty() { "ok" } else { "medium" },
            &format!("max_bus_factor_risk = {bus_risk_limit}"),
            "Require review notes, ownership backup, or tests before large changes in these modules.",
        ),
        what_if_scenario(
            "scope_pollution_guard",
            "Would this scope stay clean if root pollution were disallowed?",
            pollution.is_empty(),
            &pollution,
            if pollution.is_empty() { "ok" } else { "high" },
            "governed_scope = explicit",
            "Keep scanning the root, but keep dependency, generated, and bundled asset code outside governed source metrics.",
        ),
    ];

    let failed: Vec<&Value> = scenarios
        .iter()
        .filter(|s| s["pass"] == json!(false))
        .collect();
    let primary = failed
        .first()
        .and_then(|s| s["name"].as_str())
        .unwrap_or("none")
        .to_string();

    let mut recommendations: Vec<String> = Vec::new();
    if !rule_hints.exists {
        recommendations
            .push("Add .sentrux/rules.toml before treating this scope as governed.".to_string());
    }
    for scenario in failed.iter().take(5) {
        recommendations.push(format!(
            "{}: {}",
            scenario["name"].as_str().unwrap_or(""),
            scenario["action"].as_str().unwrap_or("")
        ));
    }
    if recommendations.is_empty() {
        recommendations.push(
            "Current scope passes the default what-if gates; keep using session_start/session_end for drift."
                .to_string(),
        );
    }

    let passing = scenarios.len() - failed.len();
    let failing = failed.len();

    Ok(json!({
        "tool": "what_if",
        "path": repo.display().to_string(),
        "generated_at": iso8601_now(),
        "rules": {
            "exists": rule_hints.exists,
            "path": rule_hints.path.display().to_string(),
            "constraints": {
                "max_cycles": rule_hints.max_cycles,
                "max_coupling": rule_hints.max_coupling,
                "max_cc": rule_hints.max_cc,
                "no_god_files": rule_hints.no_god_files,
                "ignore_test_dependencies": rule_hints.ignore_test_dependencies,
                "pollution_exclusions": rule_hints.pollution_exclusions,
            },
        },
        "thresholds": {
            "max_cc": max_cc,
            "strict_max_cc": strict_cc,
            "hard_max_cc": hard_cc,
            "max_module_coupling": coupling_limit,
            "max_blast_radius": blast_limit,
            "god_file_loc": god_loc_limit,
            "god_file_functions": god_function_limit,
            "max_bus_factor_risk": bus_risk_limit,
        },
        "summary": {
            "scenarios": scenarios.len(),
            "passing": passing,
            "failing": failing,
            "primary_risk": primary,
            "scope_candidates": scope_candidates_value,
            "source_scope": dsm["scope"],
        },
        "scenarios": scenarios,
        "recommendations": recommendations,
        "assumptions": [
            "This is deterministic static analysis, not a runtime prediction.",
            "Git-derived bus factor is conservative when files are untracked or have shallow history.",
            "Strict gates are meant for new or touched code unless the team explicitly schedules legacy cleanup.",
        ],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn init_repo(label: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "code-intel-sentrux-what-if-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("src")).expect("src dir");
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&root)
            .output()
            .expect("git init");
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&root)
            .output()
            .expect("git config email");
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&root)
            .output()
            .expect("git config name");
        fs::write(
            root.join("src/lib.rs"),
            "pub fn sample() { if true { println!(\"x\"); } }\n",
        )
        .expect("write source");
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&root)
            .output()
            .expect("git add");
        Command::new("git")
            .args(["commit", "-q", "-m", "initial"])
            .current_dir(&root)
            .output()
            .expect("git commit");
        root
    }

    #[test]
    fn what_if_evaluates_scenarios_and_reports_scope_pollution() {
        let repo = init_repo("what-if");
        fs::create_dir_all(repo.join(".sentrux")).expect("rules dir");
        fs::write(
            repo.join(".sentrux/rules.toml"),
            "max_cc = 1\nmax_coupling = \"A\"\n",
        )
        .expect("rules");
        fs::create_dir_all(repo.join("vendor")).expect("vendor dir");
        let doc = what_if(&repo).expect("what_if should run against a real git repo");
        assert_eq!(doc["tool"], "what_if");
        assert_eq!(doc["scenarios"].as_array().unwrap().len(), 8);
        assert_eq!(doc["thresholds"]["max_cc"], 1);
        assert_eq!(doc["summary"]["failing"].as_i64().unwrap() >= 1, true);
        let pollution_scenario = doc["scenarios"]
            .as_array()
            .unwrap()
            .iter()
            .find(|scenario| scenario["name"] == "scope_pollution_guard")
            .expect("pollution scenario present");
        assert_eq!(pollution_scenario["pass"], false);
        fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn coupling_grade_letters_map_to_expected_limits() {
        assert_eq!(coupling_grade_to_limit(Some("A")), 2);
        assert_eq!(coupling_grade_to_limit(Some("b")), 5);
        assert_eq!(coupling_grade_to_limit(Some("7")), 7);
        assert_eq!(coupling_grade_to_limit(None), 5);
    }
}
