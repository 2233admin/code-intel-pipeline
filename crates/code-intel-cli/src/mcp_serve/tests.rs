use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use super::{handlers, tools, ServeContext};

/// A context pointing at a real directory with no committed run.
///
/// Most guard tests want exactly this: argument validation must refuse before
/// the evidence loader is ever consulted, so a context with nothing published
/// is the honest fixture. Tests that need the loader to be the thing that
/// fails read its message instead.
struct Fixture(PathBuf);

impl Fixture {
    fn create(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "code-intel-mcp-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create MCP fixture directory");
        Self(path)
    }

    fn context(&self) -> ServeContext {
        ServeContext {
            repo_path: self.0.clone(),
            repo: "fixture-repo".into(),
            artifact_root: self.0.join("artifacts"),
            manifest: None,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn request(id: i64, method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

fn call(context: &ServeContext, tool: &str, arguments: Value) -> Value {
    super::handle_line(
        context,
        &request(
            1,
            "tools/call",
            json!({"name": tool, "arguments": arguments}),
        ),
    )
    .expect("tools/call is a request, not a notification")
}

fn tool_payload(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool result carries one text block");
    serde_json::from_str(text).expect("tool payload is JSON")
}

#[test]
fn every_registered_tool_has_a_handler() {
    let fixture = Fixture::create("registry");
    let context = fixture.context();
    for name in tools::NAMES {
        let error = handlers::call(&context, name, &json!({}))
            .err()
            .unwrap_or_default();
        assert!(
            !error.starts_with("unknown tool"),
            "{name} is advertised but has no handler"
        );
    }
    assert_eq!(tools::descriptors().len(), tools::NAMES.len());
}

#[test]
fn every_descriptor_declares_a_closed_read_only_schema() {
    for descriptor in tools::descriptors() {
        let name = descriptor["name"].as_str().expect("descriptor name");
        assert!(
            tools::is_registered(name),
            "{name} is described but not registered"
        );
        assert_eq!(
            descriptor["inputSchema"]["additionalProperties"],
            json!(false),
            "{name} accepts unexpected arguments"
        );
        assert_eq!(
            descriptor["annotations"]["readOnlyHint"],
            json!(true),
            "{name} is not annotated read-only"
        );
        assert!(
            descriptor["description"]
                .as_str()
                .is_some_and(|text| text.len() > 80),
            "{name} has no usable description"
        );
    }
}

#[test]
fn notifications_are_never_answered() {
    let fixture = Fixture::create("notify");
    let context = fixture.context();
    for line in [
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
        json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "id": Value::Null})
            .to_string(),
    ] {
        assert!(
            super::handle_line(&context, &line).is_none(),
            "a notification must not produce a response: {line}"
        );
    }
}

#[test]
fn initialize_echoes_a_supported_revision_and_substitutes_an_unknown_one() {
    let fixture = Fixture::create("initialize");
    let context = fixture.context();
    let older = super::handle_line(
        &context,
        &request(1, "initialize", json!({"protocolVersion": "2024-11-05"})),
    )
    .expect("initialize response");
    assert_eq!(older["result"]["protocolVersion"], json!("2024-11-05"));

    let unknown = super::handle_line(
        &context,
        &request(2, "initialize", json!({"protocolVersion": "1999-01-01"})),
    )
    .expect("initialize response");
    assert_eq!(
        unknown["result"]["protocolVersion"],
        json!(super::PROTOCOL_VERSION)
    );
    assert_eq!(unknown["result"]["serverInfo"]["name"], json!("code-intel"));
    assert_eq!(
        unknown["result"]["capabilities"]["tools"]["listChanged"],
        json!(false)
    );
}

#[test]
fn malformed_and_unroutable_messages_answer_as_protocol_errors() {
    let fixture = Fixture::create("protocol");
    let context = fixture.context();

    let parse_error = super::handle_line(&context, "{not json").expect("parse error response");
    assert_eq!(parse_error["error"]["code"], json!(-32700));
    assert_eq!(parse_error["id"], Value::Null);

    let no_method = super::handle_line(&context, &json!({"jsonrpc": "2.0", "id": 7}).to_string())
        .expect("invalid request response");
    assert_eq!(no_method["error"]["code"], json!(-32600));

    let unknown_method =
        super::handle_line(&context, &request(8, "tools/summon", json!({}))).expect("response");
    assert_eq!(unknown_method["error"]["code"], json!(-32601));

    let unknown_tool = super::handle_line(
        &context,
        &request(9, "tools/call", json!({"name": "rm_rf", "arguments": {}})),
    )
    .expect("response");
    assert_eq!(unknown_tool["error"]["code"], json!(-32602));
}

#[test]
fn tools_list_serves_the_whole_registry() {
    let fixture = Fixture::create("list");
    let context = fixture.context();
    let response =
        super::handle_line(&context, &request(1, "tools/list", json!({}))).expect("response");
    let served = response["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(served, tools::NAMES.to_vec());
}

/// A refusal is an answer, not a transport fault.
///
/// An agent that sees a JSON-RPC error learns "this server is broken"; one
/// that sees `isError` with a readable payload learns "run the pipeline
/// first". Getting this backwards is how a working tool acquires a reputation
/// for being flaky.
#[test]
fn a_tool_that_cannot_answer_returns_an_error_result_not_a_transport_error() {
    let fixture = Fixture::create("norun");
    let context = fixture.context();
    let response = call(&context, "get_gate_verdict", json!({}));
    assert!(
        response.get("error").is_none(),
        "a missing run is not a protocol error: {response}"
    );
    assert_eq!(response["result"]["isError"], json!(true));
    let payload = tool_payload(&response);
    assert_eq!(payload["schema"], json!("code-intel-mcp-tool-error.v1"));
    assert_eq!(payload["tool"], json!("get_gate_verdict"));
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|text| !text.is_empty()),
        "a refusal must say why: {payload}"
    );
}

/// Injection coverage: a crafted argument must not reach a path the flag
/// parsers close, and must not reach a handler at all when it is not a
/// declared argument.
#[test]
fn crafted_arguments_are_refused_before_any_evidence_is_read() {
    let fixture = Fixture::create("injection");
    let context = fixture.context();

    for escape in [
        "../../../etc/passwd",
        "..\\..\\windows\\system32\\config\\sam",
        "/etc/shadow",
        "C:/Windows/System32/drivers/etc/hosts",
        "src/../../outside.rs",
    ] {
        let response = call(&context, "get_change_impact", json!({"changed": [escape]}));
        assert_eq!(
            response["result"]["isError"],
            json!(true),
            "{escape} was not refused"
        );
        let message = tool_payload(&response)["error"].to_string();
        assert!(
            message.contains("portable repository-relative path"),
            "{escape} was refused for the wrong reason: {message}"
        );
    }

    let smuggled = call(
        &context,
        "get_facts",
        json!({"type": "code_evidence.files", "repoPath": "/tmp/elsewhere"}),
    );
    assert_eq!(smuggled["result"]["isError"], json!(true));
    assert!(tool_payload(&smuggled)["error"]
        .as_str()
        .expect("error text")
        .contains("unexpected argument: repoPath"));

    let non_object = handlers::call(&context, "get_facts", &json!("--artifact-root /etc"));
    assert_eq!(
        non_object.err().as_deref(),
        Some("arguments must be a JSON object")
    );
}

#[test]
fn a_capability_that_declared_repository_mutation_would_be_refused() {
    assert!(handlers::refuse_repository_mutation(&json!(["repo_read", "process_spawn"])).is_ok());
    let refused =
        handlers::refuse_repository_mutation(&json!(["repo_read", "local_write", "repo_mutation"]));
    assert!(
        refused
            .as_ref()
            .err()
            .is_some_and(|message| message.contains("never executes a repository writer")),
        "a mutating declaration must be refused: {refused:?}"
    );
}

/// The rerun command is advice an agent will act on, so it has to be
/// runnable: a canonicalized Windows path carries the `\\?\` verbatim prefix
/// that no shell accepts, and a path with a space needs quoting.
#[test]
fn the_rerun_command_is_shell_runnable() {
    let verbatim = ServeContext {
        repo_path: PathBuf::from(r"\\?\C:\repo\project"),
        repo: "project".into(),
        artifact_root: PathBuf::from(r"C:\artifacts"),
        manifest: None,
    };
    assert_eq!(
        handlers::rerun_command(&verbatim),
        r"code-intel C:\repo\project --mode normal"
    );

    let spaced = ServeContext {
        repo_path: PathBuf::from(r"\\?\C:\my repo\project"),
        repo: "project".into(),
        artifact_root: PathBuf::from(r"C:\artifacts"),
        manifest: None,
    };
    assert_eq!(
        handlers::rerun_command(&spaced),
        "code-intel \"C:\\my repo\\project\" --mode normal"
    );
}

#[test]
fn serve_requires_a_transport_and_rejects_unknown_flags() {
    let argv = |args: &[&str]| args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();

    let no_transport = super::parse(&argv(&[])).expect_err("a transport is required");
    assert!(no_transport.contains("serve requires a transport"));

    let unknown = super::parse(&argv(&["--mcp", "--exec"])).expect_err("unknown flag");
    assert!(unknown.contains("unknown serve argument: --exec"));

    let duplicate = super::parse(&argv(&["--mcp", "--mcp"])).expect_err("duplicate transport");
    assert_eq!(duplicate, "duplicate --mcp");

    let missing_value =
        super::parse(&argv(&["--mcp", "--repo"])).expect_err("flag without a value");
    assert!(missing_value.contains("--repo requires one value"));
}

#[test]
fn serve_defaults_the_repository_name_to_the_published_directory_name() {
    let fixture = Fixture::create("defaults");
    let argv = [
        "--mcp".to_string(),
        "--repo-path".to_string(),
        fixture.0.display().to_string(),
        "--artifact-root".to_string(),
        fixture.0.join("artifacts").display().to_string(),
    ];
    let context = super::parse(&argv).expect("serve parses");
    assert_eq!(
        context.repo,
        fixture
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .expect("fixture directory name")
    );
    assert!(context.manifest.is_none());
}
