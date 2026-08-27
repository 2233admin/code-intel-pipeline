//! Port of `legacy/scripts/tests/test-project-management-support.ps1`.
//!
//! The PowerShell file stays on disk: internalization records pin it as the
//! local boundary-test digest. These tests are the CI-facing lock.

use std::fs;
use std::path::{Path, PathBuf};

const CHECKS: &[(&str, &str, &str)] = &[
    (
        "docs/project-management-support.md",
        "mattpocock/skills",
        "Project management support doc must name mattpocock/skills setup concepts.",
    ),
    (
        "docs/project-management-support.md",
        "issue tracker",
        "Project management support doc must cover issue tracker setup.",
    ),
    (
        "docs/project-management-support.md",
        "triage label",
        "Project management support doc must cover triage labels.",
    ),
    (
        "docs/project-management-support.md",
        "domain doc",
        "Project management support doc must cover domain docs.",
    ),
    (
        "docs/project-management-support.md",
        "Linear",
        "Project management support doc must include Linear.",
    ),
    (
        "docs/project-management-support.md",
        "Obsidian/LLM wiki",
        "Project management support doc must include Obsidian/LLM wiki.",
    ),
    (
        "docs/project-management-support.md",
        "not scanner runtime",
        "Project management support doc must preserve scanner runtime boundary.",
    ),
    (
        "docs/agents/issue-tracker.md",
        "DR-0007",
        "Issue tracker doc must cite the delivery-SSOT decision record.",
    ),
    (
        "docs/agents/issue-tracker.md",
        "delivery SSOT",
        "Issue tracker doc must state GitHub issues are the delivery SSOT.",
    ),
    (
        "docs/agents/issue-tracker.md",
        "Do not store tracker credentials",
        "Issue tracker doc must preserve the no-stored-credentials boundary.",
    ),
    (
        "docs/agents/triage-labels.md",
        "needs-evaluation",
        "Triage labels must include needs-evaluation.",
    ),
    (
        "docs/agents/triage-labels.md",
        "needs-reporter-response",
        "Triage labels must include needs-reporter-response.",
    ),
    (
        "docs/agents/triage-labels.md",
        "ready-for-afk-agent",
        "Triage labels must include ready-for-afk-agent.",
    ),
    (
        "docs/agents/triage-labels.md",
        "ready-for-human",
        "Triage labels must include ready-for-human.",
    ),
    (
        "docs/agents/triage-labels.md",
        "wontfix",
        "Triage labels must include wontfix.",
    ),
    (
        "docs/agents/domain.md",
        "single-context",
        "Domain doc must record single-context layout.",
    ),
    (
        "docs/agents/domain.md",
        "Obsidian/LLM wiki",
        "Domain doc must include wiki consumption rules.",
    ),
    (
        "docs/adr/0006-project-management-support-as-agent-intake.md",
        "agent intake, not scanner runtime dependency",
        "ADR 0006 must record project-management boundary.",
    ),
    (
        "CONTEXT.md",
        "Project Management Support",
        "CONTEXT.md must define Project Management Support.",
    ),
    (
        "README.md",
        "docs/project-management-support.md",
        "README must link project management support doc.",
    ),
    (
        "skills/code-intel-pipeline/SKILL.md",
        "docs/project-management-support.md",
        "skills/code-intel-pipeline/SKILL.md must link project management support doc.",
    ),
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate lives under <repo>/crates")
        .to_path_buf()
}

#[test]
fn project_management_support_docs_keep_the_intake_boundary() {
    let root = repo_root();
    for (relative, pattern, message) in CHECKS {
        let path = root.join(relative);
        assert!(path.is_file(), "Missing file: {}", path.display());
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        assert!(
            text.contains(pattern),
            "{message} ({relative} missing `{pattern}`)"
        );
    }
}
