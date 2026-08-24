//! The served tool registry.
//!
//! Kept apart from `handlers` so the wire-facing contract (names, argument
//! schemas, the prose an agent reads when deciding what to call) can be
//! reviewed as one surface. `handlers::call` and `NAMES` are held together by
//! `every_registered_tool_has_a_handler` in `tests`, so a descriptor can never
//! advertise a tool that answers "unknown tool" at call time.

use serde_json::{json, Value};

/// Every served tool, in the order an agent should reach for them: the four
/// read projections #54 specified, then the write-assist projections #58 and
/// #345 added. `plan_structural_edit` and `scan_security_findings` are last
/// deliberately — they are the only two that spawn a child process, and an
/// agent scanning this list top-down should find the cheap answers first.
pub(super) const NAMES: &[&str] = &[
    "get_gate_verdict",
    "get_facts",
    "get_evidence",
    "get_audit_status",
    "get_change_impact",
    "plan_structural_edit",
    "scan_security_findings",
];

pub(super) fn is_registered(name: &str) -> bool {
    NAMES.contains(&name)
}

pub(super) fn descriptors() -> Vec<Value> {
    vec![
        gate_verdict(),
        facts(),
        evidence(),
        audit_status(),
        change_impact(),
        structural_edit(),
        security_findings(),
    ]
}

fn gate_verdict() -> Value {
    json!({
        "name": "get_gate_verdict",
        "title": "Gate verdict",
        "description": "The committed run's gate conclusion for this repository: triage status, \
    the first failing rule, and the minimal command that reruns it. Call this before trusting that \
    the tree is green — a passing local build says nothing about the authoritative verdict. Answers \
    from the last committed run and reports whether that run is still current for this checkout.",
        "inputSchema": {
            "type": "object",
            "properties": {},
            "additionalProperties": false
        },
        "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
    })
}

fn facts() -> Value {
    json!({
        "name": "get_facts",
        "title": "Query committed facts",
        "description": "Search the committed run's verified artifacts — hotspots, import edges, \
    scorecards, coverage, symbols, doctor observations. Filter by artifact type, artifact schema, or \
    substring. Every hit carries the digest-verified artifact reference it came from. Use this \
    instead of reading artifact files off disk: the bytes here have already been checked against the \
    snapshot they were recorded under.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "type": {"type": "string", "description": "Artifact type, e.g. code_evidence.files, diagnosis.hospital, code_evidence.scorecard."},
                "artifactSchema": {"type": "string", "description": "Artifact schema id, e.g. code-intel-hospital.v1."},
                "contains": {"type": "string", "description": "Case-insensitive substring the artifact bytes must contain."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 100, "description": "Maximum matches to return (default 20)."}
            },
            "additionalProperties": false
        },
        "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
    })
}

fn evidence() -> Value {
    json!({
        "name": "get_evidence",
        "title": "Evidence chain for a finding",
        "description": "The provenance chain behind one finding, rule id, or file path: which \
    committed artifacts mention it, each one's sha256, the snapshot identity it was recorded under, \
    and the run that published it. Call this when a verdict or a finding needs to be justified to a \
    human, or when you need to know whether a claim is backed by recorded evidence at all.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "findingId": {"type": "string", "description": "Finding id, rule id, symbol, or repository-relative path to trace."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 100, "description": "Maximum evidence links to return (default 20)."}
            },
            "required": ["findingId"],
            "additionalProperties": false
        },
        "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
    })
}

fn audit_status() -> Value {
    json!({
        "name": "get_audit_status",
        "title": "Audit department status",
        "description": "The latest audit report's per-department conclusions, scores, and \
    coverage. Optionally narrowed to one department. Reports explicitly when the committed run \
    carries no audit artifact, rather than implying a clean audit that never ran.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "department": {"type": "string", "description": "Department id to narrow to; omit for every department."}
            },
            "additionalProperties": false
        },
        "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
    })
}

fn change_impact() -> Value {
    json!({
        "name": "get_change_impact",
        "title": "Blast radius and test selection",
        "description": "Given the files you are about to change (or just changed), the files \
    reachable from them through the committed reverse import graph, plus candidate tests to run. \
    This is the mid-edit question the CLI refuses to answer, because the snapshot is never current \
    while you are typing: here the answer is served as stale-advisory, labelled with the recorded \
    and current snapshot identities. Advisory only — never gate on it.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "changed": {
                    "type": "array",
                    "items": {"type": "string"},
                    "minItems": 1,
                    "description": "Repository-relative paths you are changing, e.g. src/auth/token.rs."
                },
                "requireCurrent": {
                    "type": "boolean",
                    "description": "Refuse unless the committed snapshot still matches this checkout (default false)."
                }
            },
            "required": ["changed"],
            "additionalProperties": false
        },
        "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
    })
}

fn structural_edit() -> Value {
    json!({
        "name": "plan_structural_edit",
        "title": "Preview a structural rewrite",
        "description": "Run an ast-grep pattern across the checkout and return every match, with \
    the rewritten text when a rewrite is supplied. Preview only: nothing is written to the \
    repository. Call this before a mechanical multi-file rewrite so you edit from a match list \
    rather than from guesses, then apply the changes yourself.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "language": {"type": "string", "description": "ast-grep language id, e.g. rust, ts, python."},
                "pattern": {"type": "string", "description": "ast-grep pattern to match."},
                "rewrite": {"type": "string", "description": "Optional ast-grep rewrite template; matches are previewed, never written."},
                "paths": {
                    "type": "array",
                    "items": {"type": "string"},
                    "maxItems": 64,
                    "description": "Repository-relative paths to search (default the whole checkout)."
                }
            },
            "required": ["language", "pattern"],
            "additionalProperties": false
        },
        "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
    })
}

fn security_findings() -> Value {
    json!({
        "name": "scan_security_findings",
        "title": "Scan for security findings",
        "description": "Run the bundled, originally-authored ast-grep security rule set for one \
    language (csharp, go, java, javascript, python, rust, typescript) across the checkout and \
    return every finding. Advisory only: findings are review prompts, never a verified \
    vulnerability, and never change get_gate_verdict.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "language": {"type": "string", "description": "Bundled rule-set language id: csharp, go, java, javascript, python, rust, or typescript."},
                "paths": {
                    "type": "array",
                    "items": {"type": "string"},
                    "maxItems": 64,
                    "description": "Repository-relative paths to scan (default the whole checkout)."
                }
            },
            "required": ["language"],
            "additionalProperties": false
        },
        "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
    })
}
