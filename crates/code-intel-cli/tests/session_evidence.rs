mod common;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

static SEQ: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-session-evidence-{}-{nonce}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join("src/hot.rs"), "fn hot() {}\n").unwrap();
    let trace = root.join("trace.json");
    let trace_value = json!({
        "version":1,
        "session":{
            "id":"private-session-id",
            "harness":"Codex Desktop private-label",
            "cwd":repo,
            "eventCount":3,
            "title":"SENTINEL_PRIVATE_TITLE",
            "path":"C:/Users/private/session.jsonl"
        },
        "events":[
            {
                "seq":0,
                "tool":"exec_command",
                "action":"verify",
                "targets":[],
                "outside":[],
                "resultBytes":10,
                "isError":false,
                "summary":"SENTINEL_PRIVATE_VERIFY_COMMAND"
            },
            {
                "seq":1,
                "tool":"apply_patch",
                "action":"edit",
                "targets":[
                    {"path":"src\\hot.rs","touch":"edit"},
                    {"path":"..\\outside.txt","touch":"read"}
                ],
                "outside":[{"scope":"home","path":"C:/Users/private/secret.txt"}],
                "resultBytes":20,
                "isError":false,
                "summary":"SENTINEL_PRIVATE_EDIT_COMMAND"
            },
            {
                "seq":2,
                "tool":"wait_agent",
                "action":"other",
                "targets":[],
                "outside":[],
                "resultBytes":0,
                "isError":true,
                "summary":"SENTINEL_PRIVATE_ERROR"
            }
        ],
        "marks":[{"seq":1,"type":"user-message","note":"SENTINEL_PRIVATE_PROMPT"}],
        "stats":{
            "edited":1,
            "observability":{"reads":"estimated","errors":"exact"}
        }
    });
    fs::write(&trace, serde_json::to_vec(&trace_value).unwrap()).unwrap();
    let hotspots = root.join("hotspots.json");
    fs::write(
        &hotspots,
        serde_json::to_vec(&json!({
            "files":[{
                "path":"src/hot.rs",
                "maxComplexity":24,
                "avgComplexity":8.0,
                "loc":40,
                "git":{"churn":7,"dirty":true}
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    (repo, trace, hotspots)
}

fn run(
    repo: &Path,
    trace: &Path,
    hotspots: Option<&Path>,
    out: Option<&Path>,
) -> std::process::Output {
    let mut command = common::cli();
    command.args([
        "provider",
        "session-adapt",
        "--repo",
        repo.to_str().unwrap(),
        "--trace",
        trace.to_str().unwrap(),
    ]);
    if let Some(hotspots) = hotspots {
        command.args(["--hotspots", hotspots.to_str().unwrap()]);
    }
    if let Some(out) = out {
        command.args(["--out", out.to_str().unwrap()]);
    }
    command.output().unwrap()
}

#[test]
fn normalizes_private_trace_and_joins_structural_evidence() {
    let root = Temp::new();
    let (repo, trace, hotspots) = fixture(&root.0);
    let output = run(&repo, &trace, Some(&hotspots), None);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let artifact: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(artifact["schema"], "code-intel-session-evidence.v1");
    assert_eq!(artifact["status"], "partial");
    assert_eq!(artifact["reviewAuthority"], "advisory_only");
    assert_eq!(artifact["source"]["harness"], "codex");
    assert_eq!(artifact["summary"]["matchedTargets"], 1);
    assert_eq!(artifact["summary"]["unsafeOrOutsideTargets"], 2);
    assert_eq!(artifact["events"][1]["targets"][0]["path"], "src/hot.rs");
    assert_eq!(
        artifact["events"][1]["targets"][0]["structural"]["maxComplexity"],
        24
    );
    assert!(artifact["signals"]
        .as_array()
        .unwrap()
        .iter()
        .any(|signal| { signal["kind"] == "unverified_structural_attention_edit" }));

    let rendered = serde_json::to_string(&artifact).unwrap();
    for private in [
        "private-session-id",
        "SENTINEL_PRIVATE_TITLE",
        "SENTINEL_PRIVATE_VERIFY_COMMAND",
        "SENTINEL_PRIVATE_EDIT_COMMAND",
        "SENTINEL_PRIVATE_ERROR",
        "SENTINEL_PRIVATE_PROMPT",
        "private-label",
        "C:/Users/private",
    ] {
        assert!(!rendered.contains(private), "leaked {private}");
    }
    assert_eq!(artifact["privacy"]["userMessageMarksConsumed"], false);
    assert_eq!(artifact["privacy"]["eventSummariesConsumed"], false);
    assert_eq!(artifact["privacy"]["absolutePathsEmitted"], false);
}

#[test]
fn optional_enrichment_stays_unknown_and_output_is_non_overwriting() {
    let root = Temp::new();
    let (repo, trace, _) = fixture(&root.0);
    let out = root.0.join("session-evidence.json");
    let first = run(&repo, &trace, None, Some(&out));
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let artifact: Value = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(artifact["summary"]["matchedTargets"], 0);
    assert_eq!(
        artifact["events"][1]["targets"][0]["structural"]["status"],
        "unknown"
    );

    let second = run(&repo, &trace, None, Some(&out));
    assert_eq!(second.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&second.stderr).contains("output already exists"));
}
#[test]
fn unsupported_trace_is_rejected_without_echoing_provider_content() {
    let root = Temp::new();
    let repo = root.0.join("repo");
    fs::create_dir(&repo).unwrap();
    let trace = root.0.join("bad.json");
    fs::write(
        &trace,
        serde_json::to_vec(&json!({
            "version":2,
            "secret":"SENTINEL_DO_NOT_ECHO"
        }))
        .unwrap(),
    )
    .unwrap();
    let output = run(&repo, &trace, None, None);
    assert_eq!(output.status.code(), Some(65));
    let rendered = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!rendered.contains("SENTINEL_DO_NOT_ECHO"));
}

#[test]
fn exhaustive_hotspots_producer_lets_session_adapt_match_a_file_past_the_old_top_30_cut() {
    // Regression for issue #361: the only prior producer of
    // sentrux-hotspots.json (legacy/run-code-intel.ps1) truncated to the
    // top 30 files by complexity, so session-adapt's --hotspots join
    // silently missed any target below that cut. This spawns the real
    // `code-intel sentrux hotspots` CLI against a real 35-file repo, writes
    // its real stdout to disk, then feeds that real artifact into a real
    // `session-adapt` invocation touching the single lowest-complexity file
    // -- the one an old top-30 cut would have dropped -- end to end.
    let root = Temp::new();
    let repo = root.0.join("repo").join("src");
    fs::create_dir_all(&repo).unwrap();
    let repo = repo.parent().unwrap().to_path_buf();
    const FILE_COUNT: usize = 35;
    for index in 0..FILE_COUNT {
        // Descending branch count -> descending complexity: file 0 is the
        // most complex, file 34 (rank 35th, past any top-30 cut) has none.
        let branches = FILE_COUNT - 1 - index;
        let mut body = String::from("fn f(n: i32) -> i32 {\n    let mut total = n;\n");
        for branch in 0..branches {
            body.push_str(&format!("    if total > {branch} {{ total -= 1; }}\n"));
        }
        body.push_str("    total\n}\n");
        fs::write(repo.join("src").join(format!("file_{index:02}.rs")), body).unwrap();
    }

    let hotspots_output = common::cli()
        .args(["sentrux", "hotspots", repo.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert_eq!(
        hotspots_output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&hotspots_output.stderr)
    );
    let hotspots_doc: Value = serde_json::from_slice(&hotspots_output.stdout).unwrap();
    let files = hotspots_doc["files"].as_array().unwrap();
    assert_eq!(
        files.len(),
        FILE_COUNT,
        "hotspots producer must not drop files below any top-N cutoff"
    );
    let hotspots_path = root.0.join("real-hotspots.json");
    fs::write(&hotspots_path, &hotspots_output.stdout).unwrap();

    let lowest_complexity_file = "src/file_34.rs";
    let trace = root.0.join("trace.json");
    fs::write(
        &trace,
        serde_json::to_vec(&json!({
            "version":1,
            "session":{"id":"s","harness":"claude-code","cwd":repo,"eventCount":1},
            "events":[{
                "seq":0,
                "tool":"Read",
                "action":"read",
                "targets":[{"path":lowest_complexity_file,"touch":"read"}],
                "outside":[],
                "resultBytes":10,
                "isError":false,
                "summary":"Read"
            }],
            "stats":{"edited":0,"observability":{"reads":"exact","errors":"exact"}}
        }))
        .unwrap(),
    )
    .unwrap();

    let output = run(&repo, &trace, Some(&hotspots_path), None);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let artifact: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        artifact["summary"]["matchedTargets"], 1,
        "the 35th-ranked file must join through the real producer's real output, not just an in-memory projection"
    );
    assert_eq!(artifact["summary"]["unmatchedTargets"], 0);
    assert_eq!(
        artifact["events"][0]["targets"][0]["structural"]["status"],
        "matched"
    );
}
