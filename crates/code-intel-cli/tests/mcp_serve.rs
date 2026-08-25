//! Process-level contract for `code-intel serve --mcp`.
//!
//! The unit tests in `src/mcp_serve/tests.rs` cover dispatch and the argument
//! guards in-process. What they cannot cover is the wiring: that the route
//! table reaches the module, that stdio framing survives a real pipe, and that
//! a client's session ends cleanly when it closes stdin. Those only fail as a
//! spawned process, which is how every client will run this.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::Stdio;

use serde_json::{json, Value};

mod common;

/// A directory that exists but publishes nothing.
///
/// The handshake and framing are what this file tests, and they must hold
/// before any run has been committed — that is precisely the state an agent
/// meets on the first day in a new repository.
struct Fixture(PathBuf);

impl Fixture {
    fn create(label: &str) -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "code-intel-serve-{label}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(path.join("artifacts")).expect("create serve fixture");
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Feed the server a session and collect every response line.
///
/// Spawned through `common::cli()` so every pipeline-owned variable is cleared
/// from the same list the binary declares. Clearing a hand-picked pair here
/// instead would leave the rest of `PIPELINE_VARS` free to point this at the
/// developer's real installation and make the test pass for the wrong reason.
fn session(fixture: &Fixture, requests: &[Value]) -> (Vec<Value>, i32, String) {
    session_with_artifact_root(fixture, &fixture.0.join("artifacts"), requests)
}

fn session_with_artifact_root(
    fixture: &Fixture,
    artifact_root: &std::path::Path,
    requests: &[Value],
) -> (Vec<Value>, i32, String) {
    let mut child = common::cli()
        .args(["serve", "--mcp", "--repo-path"])
        .arg(&fixture.0)
        .args(["--artifact-root"])
        .arg(artifact_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn code-intel serve --mcp");

    {
        let stdin = child.stdin.as_mut().expect("serve stdin");
        for request in requests {
            writeln!(stdin, "{request}").expect("write MCP request");
        }
    }
    // Dropping stdin is the session's end-of-input; the server must exit 0.
    drop(child.stdin.take());

    let output = child.wait_with_output().expect("await serve");
    let responses = BufReader::new(output.stdout.as_slice())
        .lines()
        .map(|line| line.expect("read MCP response line"))
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(&line).expect("each response line is JSON"))
        .collect();
    (
        responses,
        output.status.code().expect("serve exit code"),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[test]
fn a_client_session_handshakes_lists_tools_and_closes_cleanly() {
    let fixture = Fixture::create("session");
    let (responses, exit, stderr) = session(
        &fixture,
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "protocolVersion":"2025-06-18","capabilities":{},
                "clientInfo":{"name":"contract-test","version":"1"}}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
            json!({"jsonrpc":"2.0","id":3,"method":"ping","params":{}}),
        ],
    );

    assert_eq!(exit, 0, "closing stdin ends the session cleanly: {stderr}");
    assert!(
        stderr.is_empty(),
        "a clean session writes no stderr: {stderr}"
    );
    assert_eq!(
        responses.len(),
        3,
        "four messages, one of them a notification: {responses:?}"
    );

    assert_eq!(responses[0]["id"], json!(1));
    assert_eq!(responses[0]["jsonrpc"], json!("2.0"));
    assert_eq!(
        responses[0]["result"]["serverInfo"]["name"],
        json!("code-intel")
    );
    assert_eq!(
        responses[0]["result"]["protocolVersion"],
        json!("2025-06-18")
    );
    assert_eq!(
        responses[0]["result"]["repositoryBinding"]["status"],
        json!("degraded")
    );
    assert!(responses[0]["result"]["repositoryBinding"]["reason"]
        .as_str()
        .is_some_and(|reason| reason.contains("no committed")));

    let tools = responses[1]["result"]["tools"]
        .as_array()
        .expect("tools/list serves an array");
    let names = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "get_gate_verdict",
            "get_facts",
            "get_evidence",
            "get_audit_status",
            "get_change_impact",
            "plan_structural_edit",
            "scan_security_findings",
        ]
    );
    for tool in tools {
        assert_eq!(
            tool["annotations"]["readOnlyHint"],
            json!(true),
            "{} is served without a read-only annotation",
            tool["name"]
        );
    }

    assert_eq!(responses[2]["result"], json!({}), "ping answers empty");
}

/// Before the first run there is nothing to serve, and the server has to say
/// so as an answer rather than as a crash — an agent that gets a dead process
/// on day one never calls the tool again.
#[test]
fn tools_refuse_readably_when_no_run_has_been_committed() {
    let fixture = Fixture::create("norun");
    let (responses, exit, stderr) = session(
        &fixture,
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"get_gate_verdict","arguments":{}}}),
        ],
    );

    assert_eq!(exit, 0, "a refusal is not a process failure: {stderr}");
    assert_eq!(responses.len(), 1);
    let result = &responses[0]["result"];
    assert!(
        responses[0].get("error").is_none(),
        "a missing run is not a transport error: {:?}",
        responses[0]
    );
    assert_eq!(result["isError"], json!(true));
    let payload: Value = serde_json::from_str(
        result["content"][0]["text"]
            .as_str()
            .expect("one text block"),
    )
    .expect("tool payload is JSON");
    assert_eq!(payload["schema"], json!("code-intel-mcp-tool-error.v1"));
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|text| text.contains("no committed authoritative run is indexed")),
        "the refusal must name the missing precondition: {payload}"
    );
    let cli = common::cli()
        .arg("query")
        .arg(&fixture.0)
        .args(["--kind", "evidence", "--json"])
        .env("CODE_INTEL_ARTIFACT_ROOT", fixture.0.join("artifacts"))
        .output()
        .expect("run CLI query against the same empty project");
    assert_eq!(cli.status.code(), Some(65));
    assert!(cli.stderr.is_empty());
    let cli_error: Value = serde_json::from_slice(&cli.stdout).expect("CLI project error is JSON");
    assert_eq!(
        payload["projectError"], cli_error,
        "CLI and MCP must project the same domain fault"
    );
}

#[test]
fn cli_and_mcp_preserve_usage_error_kind_and_exit_code() {
    let fixture = Fixture::create("usage-contract");
    let (responses, exit, stderr) = session(
        &fixture,
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"get_facts","arguments":{"limit":0}}}),
        ],
    );
    assert_eq!(exit, 0, "a tool refusal keeps the server alive: {stderr}");
    let mcp_error: Value = serde_json::from_str(
        responses[0]["result"]["content"][0]["text"]
            .as_str()
            .expect("MCP error text"),
    )
    .expect("MCP error payload");

    let cli = common::cli()
        .arg("query")
        .arg(&fixture.0)
        .args(["--kind", "evidence", "--limit", "0", "--json"])
        .env("CODE_INTEL_ARTIFACT_ROOT", fixture.0.join("artifacts"))
        .output()
        .expect("run CLI usage error");
    assert_eq!(cli.status.code(), Some(64));
    let cli_error: Value = serde_json::from_slice(&cli.stdout).expect("CLI usage error JSON");
    assert_eq!(
        mcp_error["projectError"], cli_error,
        "MCP must use the same typed ProjectContext query path as CLI"
    );
}

#[test]
fn cli_and_mcp_preserve_host_io_faults_exactly() {
    let fixture = Fixture::create("host-io-contract");
    let artifact_file = fixture.0.join("artifact-root-is-a-file");
    std::fs::write(&artifact_file, b"not a directory").expect("create host-I/O fixture");
    let (responses, exit, stderr) = session_with_artifact_root(
        &fixture,
        &artifact_file,
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"get_facts","arguments":{}}}),
        ],
    );
    assert_eq!(exit, 0, "a tool refusal keeps the server alive: {stderr}");
    let mcp_error: Value = serde_json::from_str(
        responses[0]["result"]["content"][0]["text"]
            .as_str()
            .expect("MCP error text"),
    )
    .expect("MCP error payload");

    let cli = common::cli()
        .arg("query")
        .arg(&fixture.0)
        .args(["--kind", "evidence", "--json"])
        .env("CODE_INTEL_ARTIFACT_ROOT", &artifact_file)
        .output()
        .expect("run CLI host-I/O fault");
    assert_eq!(cli.status.code(), Some(74));
    assert!(cli.stderr.is_empty());
    let cli_error: Value = serde_json::from_slice(&cli.stdout).expect("CLI host-I/O error JSON");
    assert_eq!(cli_error["kind"], "host_io");
    assert_eq!(mcp_error["projectError"], cli_error);
}

#[test]
fn serve_without_a_transport_is_a_usage_error() {
    let output = common::cli()
        .arg("serve")
        .output()
        .expect("spawn serve without a transport");
    assert_eq!(output.status.code(), Some(64));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("serve requires a transport") && stderr.contains("--mcp"),
        "usage must name the missing transport: {stderr}"
    );
    assert!(output.stdout.is_empty(), "a usage error writes no stdout");
}
