use serde_json::{json, Value};

use crate::change_risk::git::LogCommit;
use crate::change_risk::{Endpoint, RiskError};

use super::cochange::{build_edges, Edge, Observed};
use super::render::{build_report, render_text, Report};
use super::{cluster, ChangeAgendaRequest, Unit, DEFAULT_MIN_COCHANGE};

fn commit(hash: &str, timestamp: i64, paths: &[&str]) -> LogCommit {
    LogCommit {
        hash: hash.into(),
        timestamp,
        subject: "subject".into(),
        paths: paths.iter().map(|path| (*path).to_string()).collect(),
    }
}

fn args(raw: &[&str]) -> Vec<String> {
    raw.iter().map(|value| (*value).to_string()).collect()
}

fn contract_message(error: RiskError) -> String {
    match error {
        RiskError::Contract(message) => message,
        RiskError::HostIo(message) => panic!("expected a contract error, got host io: {message}"),
    }
}

#[test]
fn edges_keep_only_pairs_at_or_above_the_threshold() {
    let commits = vec![
        commit("a1", 30, &["core.rs", "gate.rs"]),
        commit("a2", 20, &["core.rs", "gate.rs"]),
        commit("a3", 10, &["core.rs", "gate.rs"]),
        // Two co-commits only: below the default threshold, so this pair
        // must not become an edge.
        commit("b1", 9, &["core.rs", "docs.md"]),
        commit("b2", 8, &["core.rs", "docs.md"]),
    ];
    let (edges, observed) = build_edges(&commits, DEFAULT_MIN_COCHANGE);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].left, "core.rs");
    assert_eq!(edges[0].right, "gate.rs");
    assert_eq!(edges[0].co_commits, 3);
    assert_eq!(observed.commits_walked, 5);
    assert_eq!(observed.pairs_observed, 2);
    assert_eq!(observed.edges_kept, 1);
    assert_eq!(observed.wide_commits_skipped, 0);
}

#[test]
fn edge_evidence_is_newest_first_and_marked_when_truncated() {
    let commits = vec![
        commit("old", 10, &["a.rs", "b.rs"]),
        commit("newest", 40, &["a.rs", "b.rs"]),
        commit("mid", 30, &["a.rs", "b.rs"]),
        commit("older", 20, &["a.rs", "b.rs"]),
    ];
    let (edges, _) = build_edges(&commits, 2);
    assert_eq!(edges[0].co_commits, 4);
    assert_eq!(edges[0].commits, vec!["newest", "mid", "older"]);
    assert!(
        edges[0].commits_truncated,
        "a 4-commit edge exceeds the 3-hash evidence limit and must say so"
    );
}

#[test]
fn wide_sweep_commits_are_dropped_and_counted_not_silently_ignored() {
    let sweep: Vec<String> = (0..60).map(|index| format!("file{index:02}.rs")).collect();
    let sweep_paths: Vec<&str> = sweep.iter().map(String::as_str).collect();
    let commits = vec![
        commit("sweep", 50, &sweep_paths),
        commit("c1", 30, &["a.rs", "b.rs"]),
        commit("c2", 20, &["a.rs", "b.rs"]),
    ];
    let (edges, observed) = build_edges(&commits, 2);
    assert_eq!(observed.wide_commits_skipped, 1);
    assert_eq!(observed.commits_walked, 3);
    // Only the honest pair survives; the sweep's 1770 pairs never enter.
    assert_eq!(edges.len(), 1);
    assert_eq!(observed.pairs_observed, 1);
}

#[test]
fn edges_rank_strongest_first_with_a_path_tiebreak() {
    let commits = vec![
        commit("s1", 10, &["a.rs", "z.rs"]),
        commit("s2", 11, &["a.rs", "z.rs"]),
        commit("t1", 12, &["m.rs", "n.rs"]),
        commit("t2", 13, &["m.rs", "n.rs"]),
        commit("t3", 14, &["m.rs", "n.rs"]),
    ];
    let (edges, _) = build_edges(&commits, 2);
    assert_eq!(
        edges
            .iter()
            .map(|edge| (edge.left.as_str(), edge.co_commits))
            .collect::<Vec<_>>(),
        vec![("m.rs", 3), ("a.rs", 2)]
    );
}

fn edge(left: &str, right: &str) -> Edge {
    Edge {
        left: left.into(),
        right: right.into(),
        co_commits: 3,
        commits: vec!["deadbeef".into()],
        commits_truncated: false,
    }
}

#[test]
fn clustering_is_transitive_and_keeps_unreached_files_as_singletons() {
    let paths = args(&["a.rs", "b.rs", "c.rs", "lonely.rs"]);
    let edges = vec![edge("a.rs", "b.rs"), edge("b.rs", "c.rs")];
    let groups = cluster::group(&paths, &edges);
    assert_eq!(
        groups,
        vec![args(&["a.rs", "b.rs", "c.rs"]), args(&["lonely.rs"])]
    );
}

#[test]
fn clustering_order_does_not_depend_on_the_order_edges_arrive_in() {
    let paths = args(&["a.rs", "b.rs", "c.rs", "d.rs"]);
    let forward = vec![edge("a.rs", "b.rs"), edge("c.rs", "d.rs")];
    let reversed = vec![edge("c.rs", "d.rs"), edge("a.rs", "b.rs")];
    assert_eq!(
        cluster::group(&paths, &forward),
        cluster::group(&paths, &reversed)
    );
}

#[test]
fn clustering_skips_edges_naming_paths_outside_the_changed_set() {
    let paths = args(&["a.rs"]);
    let groups = cluster::group(&paths, &[edge("a.rs", "elsewhere.rs")]);
    assert_eq!(groups, vec![args(&["a.rs"])]);
}

fn unit(members: &[&str], score: f64) -> Unit {
    Unit {
        members: args(members),
        score,
        signals: json!({"diff": {"linesChanged": 10}}),
        file_rows: members
            .iter()
            .map(|path| {
                json!({
                    "path": path,
                    "insertions": 4,
                    "deletions": 1,
                    "isSourceFile": true,
                    "isTestFile": false,
                    "bugFixCommits180d": 2,
                    "churnCommits90d": 5,
                })
            })
            .collect(),
        edges: vec![edge(members[0], members.get(1).copied().unwrap_or("b.rs"))],
    }
}

fn endpoint() -> Endpoint {
    Endpoint {
        range: "base..head".into(),
        tip: "head".into(),
    }
}

fn sample_report(units: &[Unit]) -> Value {
    build_report(Report {
        repo: std::path::Path::new("/repo"),
        revspec: "base..head",
        endpoint: &endpoint(),
        anchor_unix: 1_700_000_000,
        changed_files: 3,
        units,
        observed: &Observed {
            commits_walked: 12,
            pairs_observed: 4,
            edges_kept: 1,
            wide_commits_skipped: 2,
        },
        min_cochange: DEFAULT_MIN_COCHANGE,
        warning: None,
    })
}

#[test]
fn report_ids_units_by_rank_and_pins_the_schema() {
    let units = vec![unit(&["a.rs", "b.rs"], 61.4), unit(&["z.rs"], 12.0)];
    let report = sample_report(&units);
    assert_eq!(report["schema"], "code-intel-change-agenda.v1");
    assert_eq!(report["unitCount"], 2);
    assert_eq!(report["units"][0]["id"], "unit-1");
    assert_eq!(report["units"][0]["score"], 61);
    assert_eq!(report["units"][1]["id"], "unit-2");
    assert_eq!(report["coChange"]["wideCommitsSkipped"], 2);
}

#[test]
fn unavailable_enrichment_names_the_command_that_provides_it() {
    let report = sample_report(&[]);
    for field in ["testSelection", "structuralRules"] {
        let block = &report["enrichment"][field];
        assert_eq!(block["status"], "unavailable");
        assert!(
            block["command"]
                .as_str()
                .expect("command")
                .starts_with("code-intel "),
            "{field} must name the command that answers it, not just decline"
        );
        assert!(!block["reason"].as_str().expect("reason").is_empty());
    }
}

#[test]
fn a_report_serializes_to_identical_bytes_across_builds() {
    let first = serde_json::to_string(&sample_report(&[unit(&["a.rs", "b.rs"], 61.4)])).unwrap();
    let second = serde_json::to_string(&sample_report(&[unit(&["a.rs", "b.rs"], 61.4)])).unwrap();
    assert_eq!(first, second);
}

#[test]
fn text_render_names_files_and_the_evidence_that_joined_them() {
    let text = render_text(&sample_report(&[unit(&["a.rs", "b.rs"], 61.4)]));
    assert!(text.contains("unit-1 score 61"), "{text}");
    assert!(text.contains("a.rs +4/-1"), "{text}");
    assert!(text.contains("joined: a.rs + b.rs (3 co-commits: deadbeef)"), "{text}");
    assert!(text.contains("testSelection: unavailable -> code-intel change impact"), "{text}");
}

#[test]
fn empty_diff_reports_a_warning_and_no_units() {
    let report = build_report(Report {
        repo: std::path::Path::new("/repo"),
        revspec: "base..head",
        endpoint: &endpoint(),
        anchor_unix: 0,
        changed_files: 0,
        units: &[],
        observed: &Observed::default(),
        min_cochange: DEFAULT_MIN_COCHANGE,
        warning: Some("empty_diff"),
    });
    assert_eq!(report["warning"], "empty_diff");
    assert_eq!(report["unitCount"], 0);
}

#[test]
fn parse_requires_the_agenda_subcommand_and_a_revspec() {
    let missing_subcommand = ChangeAgendaRequest::parse(&args(&["risk", "HEAD"])).unwrap_err();
    assert!(contract_message(missing_subcommand).starts_with("usage: change agenda"));
    let missing_revspec = ChangeAgendaRequest::parse(&args(&["agenda"])).unwrap_err();
    assert!(contract_message(missing_revspec).starts_with("usage: change agenda"));
}

#[test]
fn parse_rejects_unknown_flags_duplicates_and_a_second_revspec() {
    let unknown = ChangeAgendaRequest::parse(&args(&["agenda", "HEAD", "--sample", "5"]));
    assert_eq!(
        contract_message(unknown.unwrap_err()),
        "unknown change agenda argument: --sample"
    );
    let duplicate = ChangeAgendaRequest::parse(&args(&[
        "agenda", "HEAD", "--repo", "one", "--repo", "two",
    ]));
    assert_eq!(contract_message(duplicate.unwrap_err()), "duplicate --repo");
    let two_revspecs = ChangeAgendaRequest::parse(&args(&["agenda", "HEAD", "HEAD~1"]));
    assert_eq!(
        contract_message(two_revspecs.unwrap_err()),
        "only one revspec may be supplied"
    );
}

#[test]
fn parse_rejects_a_min_cochange_that_cannot_express_a_pattern() {
    let too_low = ChangeAgendaRequest::parse(&args(&["agenda", "HEAD", "--min-cochange", "1"]));
    assert!(contract_message(too_low.unwrap_err()).contains("at least 2"));
    let not_a_number =
        ChangeAgendaRequest::parse(&args(&["agenda", "HEAD", "--min-cochange", "many"]));
    assert!(contract_message(not_a_number.unwrap_err()).contains("non-negative integer"));
}

#[test]
fn parse_defaults_are_json_and_the_documented_cochange_threshold() {
    let request = ChangeAgendaRequest::parse(&args(&["agenda", "base..head"])).expect("parses");
    assert_eq!(request.revspec, "base..head");
    assert_eq!(request.min_cochange, DEFAULT_MIN_COCHANGE);
    assert!(request.repo.is_none());
    assert!(matches!(request.format, crate::change_risk::Format::Json));
}
