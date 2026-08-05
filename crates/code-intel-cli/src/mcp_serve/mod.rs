//! `code-intel serve --mcp` — the agent-native query surface (#54, and the
//! third proposal of the write-path audit in #58).
//!
//! Every other agent-facing surface in this repository answers *after* a full
//! authoritative run and *through* artifact files. That shape is correct for
//! auditing and useless while an agent is writing code: it cannot ask one
//! question and get one answer. This module is that missing plane — a stdio
//! MCP server projecting the already-committed evidence, plus the two
//! write-assist projections #58 named.
//!
//! The boundary is structural, not a convention: this server owns no
//! authority. It re-reads what `run commit` published, re-verifies the digests
//! through `committed_evidence`, and re-uses the same request types the CLI
//! parsers build — so a query that arrives over stdio traverses exactly the
//! guards a query typed at a shell does. The single capability it can execute
//! (`edit.ast-grep-plan`) is checked against its registry declaration before
//! it runs and refused if that declaration ever admits `repo_mutation`. Gate
//! verdicts stay where they were: in the CLI and CI paths. A prompt-injected
//! query string reaching this surface can therefore read, and cannot decide.

use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::artifacts;

mod handlers;
mod tools;

#[cfg(test)]
mod tests;

/// The MCP revisions this server implements — deliberately just one.
///
/// `2025-03-26` and earlier require a server to accept JSON-RPC *batches*: a
/// top-level array of requests answered by an array of responses. This
/// transport answers one object per line and has no batch dispatch, so
/// advertising those revisions would promise something a client could hang
/// waiting for. `2025-06-18` removed batching, which is exactly why it is the
/// one revision this framing can honestly claim.
///
/// A client asking for anything else is answered with this revision, which is
/// what the specification prescribes: the client then decides whether it can
/// proceed. That is a visible negotiation failure rather than a silent one.
const PROTOCOL_VERSION: &str = "2025-06-18";
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[PROTOCOL_VERSION];

const USAGE: &str = "usage: serve --mcp [--repo-path <checkout>] [--repo <name>] [--artifact-root <root>] [--manifest <integrations.json>]";

const INSTRUCTIONS: &str =
    "Read-only projection of this repository's committed Code Intel evidence. \
Ask get_gate_verdict before trusting a green tree, get_change_impact before editing files, and \
plan_structural_edit before a mechanical multi-file rewrite. Every answer names the run and \
snapshot identity it came from; treat a stale-advisory freshness as advice, never as a gate.";

/// Where a served answer came from and which checkout it is being compared
/// against. Resolved once at startup so no per-call argument can redirect the
/// server at another repository — the client chooses tools, never targets.
#[derive(Debug)]
pub(super) struct ServeContext {
    pub(super) repo_path: PathBuf,
    pub(super) repo: String,
    pub(super) artifact_root: PathBuf,
    pub(super) manifest: Option<PathBuf>,
}

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let context = match parse(raw) {
        Ok(context) => context,
        Err(message) => {
            eprintln!("{message}");
            return 64;
        }
    };
    let stdin = io::stdin();
    let stdout = io::stdout();
    match serve(&context, &mut stdin.lock(), &mut stdout.lock()) {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("{message}");
            74
        }
    }
}

fn parse(raw: &[String]) -> Result<ServeContext, String> {
    let mut transport = false;
    let mut repo_path: Option<PathBuf> = None;
    let mut repo: Option<String> = None;
    let mut artifact_root: Option<PathBuf> = None;
    let mut manifest: Option<PathBuf> = None;
    let mut index = 0;
    while index < raw.len() {
        let flag = raw[index].as_str();
        if flag == "--mcp" {
            if transport {
                return Err("duplicate --mcp".into());
            }
            transport = true;
            index += 1;
            continue;
        }
        if !matches!(
            flag,
            "--repo-path" | "--repo" | "--artifact-root" | "--manifest"
        ) {
            return Err(format!("unknown serve argument: {flag}\n{USAGE}"));
        }
        let value = raw
            .get(index + 1)
            .filter(|value| !value.is_empty() && !value.starts_with("--"))
            .ok_or_else(|| format!("{flag} requires one value"))?;
        match flag {
            "--repo-path" => set_once(&mut repo_path, PathBuf::from(value), flag)?,
            "--repo" => set_once(&mut repo, value.clone(), flag)?,
            "--artifact-root" => set_once(&mut artifact_root, PathBuf::from(value), flag)?,
            "--manifest" => set_once(&mut manifest, PathBuf::from(value), flag)?,
            _ => unreachable!("serve flags are matched above"),
        }
        index += 2;
    }
    if !transport {
        return Err(format!("serve requires a transport\n{USAGE}"));
    }
    let repo_path = match repo_path {
        Some(path) => path,
        None => {
            env::current_dir().map_err(|error| format!("resolve working directory: {error}"))?
        }
    };
    if !repo_path.is_dir() {
        return Err(format!(
            "--repo-path is not a directory: {}",
            repo_path.display()
        ));
    }
    let repo_path = fs::canonicalize(&repo_path)
        .map_err(|error| format!("resolve --repo-path {}: {error}", repo_path.display()))?;
    // The artifact-index key defaults to the checkout's directory name because
    // that is the name `run commit` publishes under. A worktree whose folder
    // name differs from the published repository name must say so with
    // `--repo`; guessing would silently answer from another repository's runs.
    let repo = match repo {
        Some(repo) => repo,
        None => repo_path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or("--repo-path has no usable directory name; pass --repo")?
            .to_string(),
    };
    let artifact_root = match artifact_root {
        Some(root) => root,
        None => artifacts::resolve_artifact_root(None)
            .map_err(|error| format!("resolve artifact root: {error}"))?,
    };
    Ok(ServeContext {
        repo_path,
        repo,
        artifact_root,
        manifest,
    })
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), String> {
    if slot.replace(value).is_some() {
        Err(format!("duplicate {flag}"))
    } else {
        Ok(())
    }
}

/// The stdio transport: newline-delimited JSON-RPC, one message per line.
///
/// A malformed line is answered and the session continues. Only a broken pipe
/// or an unreadable stdin ends the loop with a host-IO exit, because a client
/// that disconnects mid-session is the normal way this process dies.
fn serve(
    context: &ServeContext,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<(), String> {
    let mut line = String::new();
    loop {
        line.clear();
        let read = input
            .read_line(&mut line)
            .map_err(|error| format!("read MCP stdin: {error}"))?;
        if read == 0 {
            return Ok(());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(response) = handle_line(context, trimmed) else {
            continue;
        };
        writeln!(
            output,
            "{}",
            serde_json::to_string(&response).expect("MCP response serializes")
        )
        .map_err(|error| format!("write MCP stdout: {error}"))?;
        output
            .flush()
            .map_err(|error| format!("flush MCP stdout: {error}"))?;
    }
}

/// One request in, at most one response out.
///
/// `None` means the message was a notification. JSON-RPC forbids answering
/// those, and a client that receives an unsolicited response for
/// `notifications/initialized` treats the session as broken.
pub(super) fn handle_line(context: &ServeContext, line: &str) -> Option<Value> {
    let message: Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(error) => {
            return Some(failure(
                Value::Null,
                -32700,
                &format!("parse error: {error}"),
            ))
        }
    };
    // A batch is refused out loud. It has no `id` of its own, so falling
    // through to the notification branch below would drop it silently and
    // leave the client waiting for responses that are never coming — the exact
    // hang that advertising a batching revision would have caused.
    if message.is_array() {
        return Some(failure(
            Value::Null,
            -32600,
            "invalid request: JSON-RPC batches are not supported; this server speaks MCP \
             2025-06-18, which sends messages individually",
        ));
    }
    let id = message.get("id").filter(|id| !id.is_null()).cloned()?;
    let Some(method) = message["method"].as_str() else {
        return Some(failure(id, -32600, "invalid request: no method"));
    };
    Some(dispatch(context, id, method, &message["params"]))
}

fn dispatch(context: &ServeContext, id: Value, method: &str, params: &Value) -> Value {
    match method {
        "initialize" => success(id, initialize_result(params)),
        "ping" => success(id, json!({})),
        "tools/list" => success(id, json!({ "tools": tools::descriptors() })),
        "tools/call" => tools_call(context, id, params),
        // Declared empty rather than unimplemented: a client that probes these
        // during handshake should see "this server has none", not an error it
        // may surface to the user as a failed connection.
        "resources/list" => success(id, json!({ "resources": [] })),
        "resources/templates/list" => success(id, json!({ "resourceTemplates": [] })),
        "prompts/list" => success(id, json!({ "prompts": [] })),
        other => failure(id, -32601, &format!("method not found: {other}")),
    }
}

fn initialize_result(params: &Value) -> Value {
    let requested = params["protocolVersion"]
        .as_str()
        .unwrap_or(PROTOCOL_VERSION);
    let negotiated = if SUPPORTED_PROTOCOL_VERSIONS.contains(&requested) {
        requested
    } else {
        PROTOCOL_VERSION
    };
    json!({
        "protocolVersion": negotiated,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": "code-intel", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
    })
}

/// A tool that refuses is a *result* with `isError`, not a JSON-RPC error.
///
/// The distinction is load-bearing for an agent: a JSON-RPC error is a
/// transport fault it should retry or report, while `isError` is an answer it
/// should read — "no committed run exists yet", "that path escapes the
/// repository". Collapsing the two would train agents to treat a refusal as a
/// broken server. Only an unknown tool name is a protocol error.
fn tools_call(context: &ServeContext, id: Value, params: &Value) -> Value {
    let Some(name) = params["name"].as_str() else {
        return failure(
            id,
            -32602,
            "invalid params: tools/call requires a tool name",
        );
    };
    if !tools::is_registered(name) {
        return failure(id, -32602, &format!("unknown tool: {name}"));
    }
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    match handlers::call(context, name, &arguments) {
        Ok(payload) => success(id, tool_result(&payload, false)),
        Err(message) => success(
            id,
            tool_result(
                &json!({
                    "schema": "code-intel-mcp-tool-error.v1",
                    "tool": name,
                    "error": message,
                }),
                true,
            ),
        ),
    }
}

fn tool_result(payload: &Value, is_error: bool) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string(payload).expect("tool payload serializes"),
        }],
        "isError": is_error,
    })
}

fn success(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn failure(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}
