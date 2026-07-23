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
    let mut command = Command::new(env!("CARGO_BIN_EXE_code-intel"));
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
