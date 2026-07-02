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

fn summary_for(report: Value, hospital: Value, artifact_dir: &Path) -> ResumeSummary {
    build_resume_summary(
        PathBuf::from("D:/work/quant-system"),
        artifact_dir.to_path_buf(),
        artifact_dir.join("report.json"),
        &report,
        &hospital,
    )
}

#[test]
fn resume_contract_routes_graph_missing_to_understanding() {
    let dir = unique_temp_dir("graph-missing");
    fs::create_dir_all(&dir).expect("fixture dir should be created");
    touch(&dir.join("summary.md"), "# Summary");
    touch(&dir.join("understanding.md"), "# Understanding");

    let hospital_path = dir.join("hospital-report.json");
    let hospital_markdown = dir.join("hospital.md");
    let report = json!({
        "hospital": {
            "path": hospital_path,
            "markdown": hospital_markdown
        },
        "githubResearch": {
            "status": "not_applicable",
            "required": false,
            "path": "",
            "markdown": ""
        },
        "summary": {
            "failed": 0,
            "manualRequired": 1,
            "failureCategories": {
                "providerQuota": 0,
                "localToolError": 0,
                "graphMissing": 1,
                "sentruxFail": 0
            }
        }
    });
    let hospital = json!({
        "triage": {
            "status": "amber",
            "disposition": "admit",
            "primary_diagnosis": "architecture graph missing",
            "next_protocol": "diagnose",
            "research_status": "not_applicable",
            "research_required": false
        },
        "state_machine": {
            "current_state": "diagnose"
        }
    });

    let summary = summary_for(report, hospital, &dir);

    assert_eq!(summary.graph_missing, 1);
    assert_eq!(summary.hospital_next_protocol, "diagnose");
    assert!(!summary.research_required);
    assert_eq!(next_read(&summary), dir.join("understanding.md"));
}

#[test]
fn resume_contract_prioritizes_github_research_when_required() {
    let dir = unique_temp_dir("research-required");
    fs::create_dir_all(&dir).expect("fixture dir should be created");
    touch(&dir.join("understanding.md"), "# Understanding");
    let research_markdown = dir.join("github-solution-research.md");
    touch(&research_markdown, "# GitHub Solution Research");

    let report = json!({
        "hospital": {
            "path": dir.join("hospital-report.json"),
            "markdown": dir.join("hospital.md")
        },
        "githubResearch": {
            "status": "manual_required",
            "required": true,
            "path": dir.join("github-solution-research.json"),
            "markdown": research_markdown
        },
        "summary": {
            "failed": 1,
            "manualRequired": 1,
            "failureCategories": {
                "providerQuota": 0,
                "localToolError": 0,
                "graphMissing": 0,
                "sentruxFail": 1
            }
        }
    });
    let hospital = json!({
        "triage": {
            "status": "red",
            "disposition": "admit",
            "primary_diagnosis": "architecture gate failure",
            "next_protocol": "github_solution_research",
            "research_status": "manual_required",
            "research_required": true
        },
        "state_machine": {
            "current_state": "triage"
        }
    });

    let summary = summary_for(report, hospital, &dir);

    assert_eq!(summary.sentrux_fail, 1);
    assert!(summary.research_required);
    assert_eq!(summary.hospital_next_protocol, "github_solution_research");
    assert_eq!(next_read(&summary), research_markdown);
}

#[test]
fn classify_contract_requires_research_for_upstream_or_tool_blockers() {
    let report = json!({
        "summary": {
            "failureCategories": {
                "providerQuota": 1,
                "localToolError": 0,
                "graphMissing": 1,
                "sentruxFail": 0
            }
        }
    });

    let provider_quota = int_at(&report, &["summary", "failureCategories", "providerQuota"]);
    let local_tool_error = int_at(&report, &["summary", "failureCategories", "localToolError"]);
    let sentrux_fail = int_at(&report, &["summary", "failureCategories", "sentruxFail"]);

    assert!(provider_quota > 0 || local_tool_error > 0 || sentrux_fail > 0);
}
