use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use super::{publish_named, AdapterArtifact, AdapterError, AdapterOutput};
use crate::adapter_contract::AdapterDomainVerdict;
use crate::artifact_ref::VerifiedArtifact;

const EXCLUDED_DIRS: &[&str] = &[
    ".git",
    "target",
    "dist",
    "build",
    "vendor",
    "venv",
    ".venv",
    "node_modules",
    "__pycache__",
];
const SOURCE_EXTENSIONS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "rs", "py", "go", "ps1", "psm1", "cs", "java", "kt", "swift", "vue",
    "svelte", "v",
];

pub(crate) fn execute(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    if !verified_inputs.is_empty() {
        return Err(AdapterError::Contract(
            "workflow recommendation does not accept input artifacts".into(),
        ));
    }
    let options = request
        .get("options")
        .and_then(Value::as_object)
        .ok_or_else(|| AdapterError::InvalidOptions("options must be an object".into()))?;
    if options
        .keys()
        .any(|key| !matches!(key.as_str(), "repoPath" | "auto"))
    {
        return Err(AdapterError::InvalidOptions(
            "workflow recommendation accepts only repoPath/auto".into(),
        ));
    }
    let repo = options
        .get("repoPath")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| AdapterError::InvalidOptions("options.repoPath must be non-empty".into()))?;
    if !repo.is_dir() {
        return Err(AdapterError::InvalidOptions(format!(
            "repoPath is not a directory: {}",
            repo.display()
        )));
    }
    let auto = match options.get("auto") {
        None => false,
        Some(value) => value.as_bool().ok_or_else(|| {
            AdapterError::InvalidOptions("options.auto must be boolean when present".into())
        })?,
    };
    let result = recommend(&repo, auto)?;
    validate_proposal(&result)?;
    let bytes = serde_json::to_vec(&result).map_err(|error| {
        AdapterError::Internal(format!("serialize workflow recommendation: {error}"))
    })?;
    publish_named(out, "workflow-recommendation.json", &bytes, |_| Ok(()))?;
    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: "code-intel-advisory-workflow-recommendation.v1".into(),
            artifact_type: "advisory.workflow-recommendation".into(),
            relative_path: "workflow-recommendation.json".into(),
            bytes,
        }],
        observed_effects: vec![],
        domain_verdict: AdapterDomainVerdict::Pass,
        domain_failure: None,
    })
}

fn recommend(repo: &Path, auto: bool) -> Result<Value, AdapterError> {
    let (files, lines) = source_metrics(repo);
    let governance = governance(repo);
    let collaboration = collaboration(repo);
    let cicd_score = cicd_score(repo);
    let has_tests = has_tests(repo);

    let spec = spec_recommendation(
        files,
        lines,
        &governance,
        &collaboration,
        cicd_score,
        has_tests,
    );
    let matt = matt_recommendation(files, lines, &governance, &collaboration);
    let gstack = gstack_recommendation(repo, &collaboration);
    let alternatives = vec![matt, gstack, spec.clone()];
    let confidence =
        if spec["verdict"] == "already_adopted" || spec["score"].as_i64().unwrap_or(0) >= 70 {
            "high"
        } else if spec["score"].as_i64().unwrap_or(0) >= 30 {
            "medium"
        } else {
            "low"
        };
    let evidence = json!([
        {"kind":"repository-metrics", "value":format!("files={files};lines={lines};repoAgeDays={}", collaboration["repoAgeDays"])},
        {"kind":"governance", "value":format!("openSpec={};specKit={};tests={has_tests};cicdScore={cicd_score}", governance["hasOpenSpec"], governance["hasSpecKit"])}
    ]);
    Ok(json!({
        "schema":"code-intel-advisory-workflow-recommendation.v1",
        "kind":"proposal",
        "recommendation": {
            "candidate": spec["candidate"], "stack": spec["stack"], "verdict": spec["verdict"],
            "score": spec["score"], "reasons": spec["reasons"], "entrySkills": spec["entrySkills"],
            "brief": spec["brief"]
        },
        "evidence": evidence,
        "confidence": confidence,
        "alternatives": alternatives,
        "provenance": {"capabilityId":"advisory.workflow-recommend", "implementation":"rust.workflow_recommendation", "repository":repo, "compatibilityOptions":{"auto":auto}},
        "effects": []
    }))
}

fn source_metrics(repo: &Path) -> (usize, usize) {
    let mut files = 0;
    let mut lines = 0;
    visit_files(repo, &mut |path| {
        if path
            .extension()
            .and_then(|v| v.to_str())
            .is_some_and(|ext| SOURCE_EXTENSIONS.contains(&ext))
        {
            files += 1;
            if let Ok(bytes) = fs::read(path) {
                lines += bytes.iter().filter(|byte| **byte == b'\n').count()
                    + usize::from(!bytes.is_empty() && !bytes.ends_with(b"\n"));
            }
        }
    });
    (files, lines)
}

fn visit_files(root: &Path, visit: &mut impl FnMut(&Path)) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name();
        if EXCLUDED_DIRS.iter().any(|excluded| name == *excluded) {
            continue;
        }
        if file_type.is_dir() {
            visit_files(&path, visit);
        } else if file_type.is_file() {
            visit(&path);
        }
    }
}

fn validate_proposal(value: &Value) -> Result<(), AdapterError> {
    let object = value.as_object().ok_or_else(|| {
        AdapterError::Contract("workflow recommendation must be an object".into())
    })?;
    let expected = [
        "schema",
        "kind",
        "recommendation",
        "evidence",
        "confidence",
        "alternatives",
        "provenance",
        "effects",
    ];
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(AdapterError::Contract(
            "workflow recommendation top-level contract is not exact".into(),
        ));
    }
    if value["schema"] != "code-intel-advisory-workflow-recommendation.v1"
        || value["kind"] != "proposal"
        || !matches!(
            value["confidence"].as_str(),
            Some("low" | "medium" | "high")
        )
        || value["evidence"].as_array().map_or(true, Vec::is_empty)
        || value["alternatives"].as_array().map_or(true, |items| {
            items.len() < 3
                || items.iter().any(|item| {
                    item["candidate"]
                        .as_str()
                        .map_or(true, |candidate| candidate.is_empty())
                })
        })
        || value["effects"]
            .as_array()
            .map_or(true, |items| !items.is_empty())
        || value
            .pointer("/provenance/capabilityId")
            .and_then(Value::as_str)
            != Some("advisory.workflow-recommend")
    {
        return Err(AdapterError::Contract(
            "workflow recommendation violates the advisory proposal boundary".into(),
        ));
    }
    Ok(())
}

fn governance(repo: &Path) -> Value {
    json!({
        "hasDesign": repo.join("design.md").is_file(),
        "hasSpecs": repo.join("specs").is_dir(),
        "hasSecurityReview": repo.join("security-review.md").is_file() || repo.join("docs/security-review.md").is_file(),
        "hasArchitecture": repo.join("architecture.md").is_file(),
        "hasOpenSpec": repo.join("openspec").is_dir(),
        "hasSpecKit": repo.join(".specify").is_dir(),
        "hasADRs": repo.join("docs/adr").is_dir() || repo.join("adr").is_dir(),
        "hasConstitution": repo.join("constitution.md").is_file(),
        "hasIssueTemplates": repo.join(".github/ISSUE_TEMPLATE").is_dir()
    })
}

fn collaboration(repo: &Path) -> Value {
    let contributors = git_lines(repo, &["log", "--format=%ae"])
        .into_iter()
        .collect::<BTreeSet<_>>()
        .len();
    let first = git_lines(repo, &["log", "--reverse", "--format=%ct"])
        .first()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let last = git_lines(repo, &["log", "-1", "--format=%ct"])
        .first()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .unwrap_or(0);
    json!({"contributors":contributors, "repoAgeDays": if first == 0 {0} else {now.saturating_sub(first) / 86400}, "lastCommitAgeDays": if last == 0 {9999} else {now.saturating_sub(last) / 86400}})
}

fn git_lines(repo: &Path, args: &[&str]) -> Vec<String> {
    let output = Command::new("git")
        // Match the legacy detector's hardening: repository configuration must
        // not select a hook, pager, SSH command, or external diff executable.
        .args([
            "-c",
            "core.fsmonitor=",
            "-c",
            "core.hooksPath=",
            "-c",
            "core.sshCommand=",
            "-c",
            "diff.external=",
            "-c",
            "core.pager=",
        ])
        .env_remove("GIT_CONFIG_SYSTEM")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .current_dir(repo)
        .args(args)
        .output();
    output
        .ok()
        .filter(|value| value.status.success())
        .map(|value| {
            String::from_utf8_lossy(&value.stdout)
                .lines()
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn cicd_score(repo: &Path) -> i64 {
    [
        ".github/workflows",
        ".gitlab-ci.yml",
        "Jenkinsfile",
        "azure-pipelines.yml",
        ".circleci",
    ]
    .iter()
    .filter(|path| repo.join(path).exists())
    .count() as i64
        * 10
}

fn has_tests(repo: &Path) -> bool {
    let mut found = false;
    visit_files(repo, &mut |path| {
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            return;
        };
        let lower = name.to_ascii_lowercase();
        let stem = Path::new(&lower)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let relative = path.strip_prefix(repo).unwrap_or(path);
        let in_test_dir = relative.components().any(|component| {
            component
                .as_os_str()
                .to_str()
                .map(|value| {
                    matches!(
                        value.to_ascii_lowercase().as_str(),
                        "test" | "tests" | "__tests__"
                    )
                })
                .unwrap_or(false)
        });
        let test_named = stem.ends_with("_test")
            || stem.ends_with("_tests")
            || lower.contains(".spec.")
            || lower.contains(".test.");
        if in_test_dir || test_named {
            found = true;
        }
    });
    found
}

fn spec_recommendation(
    files: usize,
    lines: usize,
    governance: &Value,
    collaboration: &Value,
    cicd: i64,
    tests: bool,
) -> Value {
    if governance["hasOpenSpec"] == true {
        return spec_result(
            "openspec-opsx",
            "already_adopted",
            100,
            vec!["检测到 openspec/ 目录 (已在用 OpenSpec OPSX)".into()],
            vec![],
        );
    }
    if governance["hasSpecKit"] == true {
        return spec_result(
            "spec-kit",
            "already_adopted",
            100,
            vec!["检测到 .specify/ 目录 (已在用 spec-kit)".into()],
            vec![],
        );
    }
    let mut score = 0i64;
    let mut reasons = Vec::new();
    if lines > 50000 {
        score += 40;
        reasons.push(format!("大型代码库 ({lines} 行)"));
    } else if lines > 10000 {
        score += 25;
        reasons.push(format!("中型代码库 ({lines} 行)"));
    } else if lines > 5000 {
        score += 10;
        reasons.push(format!("较小代码库 ({lines} 行)"));
    }
    for (key, points, text) in [
        ("hasDesign", 20, "存在 design.md"),
        ("hasArchitecture", 15, "存在 architecture.md"),
        ("hasSpecs", 25, "存在 specs/ 目录"),
        ("hasSecurityReview", 25, "存在安全审查文件"),
        ("hasADRs", 15, "存在 ADR 文档"),
        ("hasConstitution", 20, "存在 constitution.md"),
    ] {
        if governance[key] == true {
            score += points;
            reasons.push(text.into());
        }
    }
    let contributors = collaboration["contributors"].as_u64().unwrap_or(0);
    let age = collaboration["repoAgeDays"].as_u64().unwrap_or(0);
    if contributors > 5 {
        score += 25;
        reasons.push(format!("多人协作 ({contributors} 人)"));
    } else if contributors > 2 {
        score += 15;
        reasons.push(format!("少量协作 ({contributors} 人)"));
    }
    if age > 365 {
        score += 10;
        reasons.push(format!("成熟项目 ({age} 天)"));
    }
    score += cicd;
    if tests {
        score += 5;
    } else {
        score -= 5;
    }
    let verdict = if score >= 50 {
        "recommended"
    } else if score >= 30 {
        "optional"
    } else {
        "not_needed"
    };
    let brownfield = files > 5 && age > 90;
    let tool = if brownfield {
        "openspec-opsx"
    } else {
        "spec-kit"
    };
    reasons.push(if brownfield {
        format!("存量项目 (files={files}, repoAgeDays={age}) -> OpenSpec OPSX")
    } else {
        format!("新建/近乎空仓 (files={files}, repoAgeDays={age}) -> spec-kit")
    });
    let entry = if verdict == "not_needed" {
        vec![]
    } else if brownfield {
        vec!["openspec init"]
    } else {
        vec!["specify init"]
    };
    spec_result(tool, verdict, score, reasons, entry)
}

fn spec_result(
    tool: &str,
    verdict: &str,
    score: i64,
    reasons: Vec<String>,
    entry: Vec<&str>,
) -> Value {
    let recommended = if verdict == "not_needed" {
        "none"
    } else {
        tool
    };
    json!({"candidate":tool,"stack":"spec-driven","verdict":verdict,"score":score,"reasons":reasons,"entrySkills":entry,"brief":{"recommended":recommended,"verdict":verdict,"confidence":if score >= 70 {"high"} else if score >= 30 {"medium"} else {"low"},"why":reasons.iter().take(6).collect::<Vec<_>>(),"whyNot":[],"doFirst":entry,"doNotDoYet":["Do not auto-run init from Code Intel Pipeline.","Do not create or update external issue trackers without explicit authorization."],"fallback":"Re-run the detector when repository scope or governance changes.","acceptance":["PRD or feature requirements are decomposed into explicit phases.","Each phase names deliverables and requirement coverage.","Tasks map to acceptance tests before implementation starts.","Completion conditions are explicit and reviewable."]}})
}

fn matt_recommendation(
    files: usize,
    lines: usize,
    governance: &Value,
    collaboration: &Value,
) -> Value {
    let active = collaboration["lastCommitAgeDays"].as_u64().unwrap_or(9999) <= 90;
    let verdict = if active && files > 5 {
        "recommended"
    } else {
        "not_needed"
    };
    let mut reasons = vec![
        if active {
            "活跃开发".into()
        } else {
            "90天内无提交".into()
        },
        if files > 5 {
            format!("在建项目 (files={files})")
        } else {
            "源码文件过少".into()
        },
    ];
    let mut skills = Vec::new();
    if verdict == "recommended" {
        if governance["hasIssueTemplates"] == true {
            skills.push("/triage");
            reasons.push("检测到 issue templates".into());
        }
        skills.push("/grill-with-docs");
        if lines > 20000 || collaboration["contributors"].as_u64().unwrap_or(0) > 2 {
            skills.extend(["/to-prd", "/to-issues"]);
        }
    }
    json!({"candidate":"matt-flow","stack":"matt-flow","verdict":verdict,"score":0,"reasons":reasons,"entrySkills":skills})
}

fn gstack_recommendation(repo: &Path, collaboration: &Value) -> Value {
    let active = collaboration["lastCommitAgeDays"].as_u64().unwrap_or(9999) <= 90;
    let verdict = if active { "recommended" } else { "not_needed" };
    let mut skills = Vec::new();
    if verdict == "recommended" {
        let web = repo.join("package.json").is_file()
            || ["frontend", "web", "ui"]
                .iter()
                .any(|d| repo.join(d).is_dir());
        let deploy = ["Dockerfile", "docker-compose.yml", "docker-compose.yaml"]
            .iter()
            .any(|p| repo.join(p).is_file());
        if web {
            skills.extend(["/qa", "/design-review"]);
        }
        if deploy {
            skills.extend(["/ship", "/canary"]);
        }
        if skills.is_empty() {
            skills.push("/review");
        }
    }
    json!({"candidate":"gstack","stack":"gstack","verdict":verdict,"score":0,"reasons":[if active {"活跃开发"} else {"90天内无提交"}],"entrySkills":skills})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ignores_excluded_directories_and_counts_rust_source() {
        let root =
            std::env::temp_dir().join(format!("workflow-recommendation-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(root.join("src/no-final-newline.rs"), "let x = 1;").unwrap();
        fs::write(root.join("target/generated.rs"), "\n\n").unwrap();
        assert_eq!(source_metrics(&root), (2, 2));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detects_conventional_test_locations_without_filename_substring_false_positives() {
        let root = std::env::temp_dir().join(format!(
            "workflow-recommendation-tests-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/contest.rs"), "fn main() {}\n").unwrap();
        assert!(!has_tests(&root));
        fs::create_dir_all(root.join("Tests")).unwrap();
        fs::write(root.join("Tests/contract.rs"), "#[test]\nfn works() {}\n").unwrap();
        assert!(has_tests(&root));
        let _ = fs::remove_dir_all(root);
    }
}
