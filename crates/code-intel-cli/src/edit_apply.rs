//! `code-intel edit apply` — the agent-facing front door for the
//! span-addressed write primitive (#96 item 1, charter gate G4 in #139).
//!
//! This route owns argument shape and rendering and nothing else. The write
//! itself goes through the `edit.span-apply` capability: this module composes
//! a `code-intel-capability-request.v1`, takes `implementation` and
//! `allowedEffects` from the registered declaration (through
//! `ExecutionPolicy`, so the effect policy is registry-declared rather than
//! route-declared), and hands it to the same executor `capability exec`
//! uses. There is deliberately no filesystem write anywhere in this file —
//! a second write path would make the envelope's guarantees optional.
//!
//! The digest is supplied by the caller and is never computed here. Computing
//! it would make verification a tautology: the point is to compare the bytes
//! the caller *believed* were at that address against the bytes that are
//! there now. A caller who does not know the digest learns it from a refusal,
//! which reports the digest and a bounded literal of what it actually found.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::{capability, capability_inventory, execution_policy, snapshot};

const CAPABILITY: &str = "edit.span-apply";

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let cli = match Cli::parse(raw) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}");
            return 64;
        }
    };
    match execute(&cli) {
        Ok((exit_code, document)) => {
            println!(
                "{}",
                serde_json::to_string(&document).expect("edit apply result serializes")
            );
            exit_code
        }
        Err((exit_code, message)) => {
            eprintln!("{message}");
            exit_code
        }
    }
}

struct Pending {
    start_line: u64,
    start_column: u64,
    end_line: u64,
    end_column: u64,
    expected_sha256: Option<String>,
    replacement: Option<String>,
}

struct Cli {
    repo_path: PathBuf,
    file: String,
    spans: Vec<Value>,
    out: Option<PathBuf>,
    manifest: Option<PathBuf>,
    envelope: bool,
}

impl Cli {
    fn parse(raw: &[String]) -> Result<Self, String> {
        let mut repo_path: Option<PathBuf> = None;
        let mut file: Option<String> = None;
        let mut out: Option<PathBuf> = None;
        let mut manifest: Option<PathBuf> = None;
        let mut envelope = false;
        let mut pending: Vec<Pending> = Vec::new();
        let mut index = 0;
        while index < raw.len() {
            let flag = raw[index].as_str();
            // The whole point of this route is that a small change costs few
            // tokens. Echoing a ~1.5 KB capability envelope on every one-
            // identifier edit would spend on the answer what the span address
            // saved on the question, so the envelope is opt-in.
            if flag == "--envelope" {
                if envelope {
                    return Err("duplicate --envelope".into());
                }
                envelope = true;
                index += 1;
                continue;
            }
            let value = raw
                .get(index + 1)
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| format!("{flag} requires exactly one value"))?;
            match flag {
                "--repo-path" => set_once(&mut repo_path, PathBuf::from(value), flag)?,
                "--file" => set_once(&mut file, value.replace('\\', "/"), flag)?,
                "--out" => set_once(&mut out, PathBuf::from(value), flag)?,
                "--manifest" => set_once(&mut manifest, PathBuf::from(value), flag)?,
                "--span" => pending.push(parse_span_token(value)?),
                "--expect-sha256" => {
                    let span = open_span(&mut pending, flag)?;
                    set_once(&mut span.expected_sha256, value.to_string(), flag)?;
                }
                "--replacement" => {
                    let span = open_span(&mut pending, flag)?;
                    set_once(&mut span.replacement, value.to_string(), flag)?;
                }
                "--replacement-file" => {
                    let text = fs::read_to_string(value)
                        .map_err(|error| format!("read --replacement-file {value}: {error}"))?;
                    let span = open_span(&mut pending, flag)?;
                    set_once(&mut span.replacement, text, flag)?;
                }
                other => return Err(format!("unknown argument for edit apply: {other}")),
            }
            index += 2;
        }
        if pending.is_empty() {
            return Err(USAGE.to_string());
        }
        let spans = pending
            .into_iter()
            .map(|span| {
                let address = format!(
                    "{}:{}-{}:{}",
                    span.start_line, span.start_column, span.end_line, span.end_column
                );
                Ok(json!({
                    "startLine": span.start_line,
                    "startColumn": span.start_column,
                    "endLine": span.end_line,
                    "endColumn": span.end_column,
                    "expectedSha256": span.expected_sha256.ok_or_else(|| format!(
                        "--span {address} has no --expect-sha256"
                    ))?,
                    "replacement": span.replacement.ok_or_else(|| format!(
                        "--span {address} has no --replacement or --replacement-file"
                    ))?,
                }))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            repo_path: repo_path.ok_or_else(|| USAGE.to_string())?,
            file: file.ok_or_else(|| USAGE.to_string())?,
            spans,
            out,
            manifest,
            envelope,
        })
    }
}

const USAGE: &str = "usage: edit apply --repo-path <checkout> --file <repo-relative-path> (--span <startLine:startColumn-endLine:endColumn> --expect-sha256 <sha256-of-current-span-bytes> --replacement <text>|--replacement-file <path>)... [--out <staging-dir>] [--manifest <integrations.json>] [--envelope]";

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), String> {
    if slot.replace(value).is_some() {
        return Err(format!("duplicate {flag}"));
    }
    Ok(())
}

fn open_span<'a>(pending: &'a mut [Pending], flag: &str) -> Result<&'a mut Pending, String> {
    pending
        .last_mut()
        .ok_or_else(|| format!("{flag} must follow a --span"))
}

/// `startLine:startColumn-endLine:endColumn`, 1-based, end column exclusive.
fn parse_span_token(token: &str) -> Result<Pending, String> {
    let malformed =
        || format!("--span must be <startLine:startColumn-endLine:endColumn>, got: {token}");
    let (start, end) = token.split_once('-').ok_or_else(malformed)?;
    let point = |value: &str| -> Result<(u64, u64), String> {
        let (line, column) = value.split_once(':').ok_or_else(malformed)?;
        Ok((
            line.parse::<u64>().map_err(|_| malformed())?,
            column.parse::<u64>().map_err(|_| malformed())?,
        ))
    };
    let (start_line, start_column) = point(start)?;
    let (end_line, end_column) = point(end)?;
    Ok(Pending {
        start_line,
        start_column,
        end_line,
        end_column,
        expected_sha256: None,
        replacement: None,
    })
}

fn execute(cli: &Cli) -> Result<(i32, Value), (i32, String)> {
    let repo = fs::canonicalize(&cli.repo_path).map_err(|error| {
        (
            65,
            format!(
                "cannot resolve --repo-path {}: {error}",
                cli.repo_path.display()
            ),
        )
    })?;
    // A mistyped path is the most common way to call this wrong, and the
    // snapshot builder reports it as "non-root scope has no manifest entries",
    // which reads like an internal fault. Say the plain thing first.
    if !repo.join(&cli.file).is_file() {
        return Err((
            65,
            format!("--file is not a file in --repo-path: {}", cli.file),
        ));
    }
    // Scope the snapshot to the one file being edited. A repository-wide
    // scope would make every keystroke-sized edit pay for a full-tree digest,
    // which is how a correct primitive becomes one nobody calls.
    let snapshot = snapshot::build_for_dag(&repo, "explicit_overlay", &[cli.file.clone()])
        .map_err(|error| (65, format!("snapshot identity for {}: {error}", cli.file)))?;
    let declaration = capability::declaration_for(CAPABILITY, cli.manifest.as_deref())
        .map_err(|error| (69, error))?;
    let policy =
        execution_policy::ExecutionPolicy::for_profile(execution_policy::RunProfile::Default);
    let request = json!({
        "schema": "code-intel-capability-request.v1",
        "capability": CAPABILITY,
        "contractVersion": 1,
        "implementation": declaration["implementation"],
        "snapshot": snapshot["snapshot"],
        "options": {
            "repoPath": repo,
            "path": cli.file,
            "spans": cli.spans,
        },
        "inputs": [],
        "effectPolicy": {"allowedEffects": policy.allowed_effects(&declaration)},
    });

    let staging = Staging::open(cli.out.clone()).map_err(|error| (74, error))?;
    let outcome = capability::exec_in_process(
        CAPABILITY,
        &request,
        staging.path(),
        cli.manifest.as_deref(),
        capability_inventory::execute,
    );
    let span_edit = outcome
        .result
        .as_ref()
        .and_then(|result| result["artifacts"][0]["path"].as_str())
        .and_then(|relative| fs::read(staging.path().join(relative)).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    Ok((
        outcome.exit_code,
        json!({
            "schema": "code-intel-edit-apply.v1",
            "capability": CAPABILITY,
            "exitCode": outcome.exit_code,
            "applied": span_edit
                .as_ref()
                .and_then(|value| value["applied"].as_bool())
                .unwrap_or(false),
            "envelope": if cli.envelope { outcome.result } else { None },
            "spanEdit": span_edit,
            "diagnostic": outcome.diagnostic,
        }),
    ))
}

/// The envelope demands an output directory it can create exclusively. A
/// caller who wants the envelope and artifact bytes kept passes `--out`;
/// otherwise they land in a per-process staging directory that is removed
/// once the payload has been read back.
struct Staging {
    path: PathBuf,
    ephemeral: bool,
}

impl Staging {
    fn open(explicit: Option<PathBuf>) -> Result<Self, String> {
        match explicit {
            Some(path) => Ok(Self {
                path,
                ephemeral: false,
            }),
            None => {
                let nonce = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|error| error.to_string())?
                    .as_nanos();
                Ok(Self {
                    path: env::temp_dir().join(format!(
                        "code-intel-edit-apply-{}-{nonce}",
                        std::process::id()
                    )),
                    ephemeral: true,
                })
            }
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if self.ephemeral {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn each_span_carries_its_own_digest_and_replacement() {
        let cli = Cli::parse(&args(&[
            "--repo-path",
            ".",
            "--file",
            "src/lib.rs",
            "--span",
            "12:21-12:33",
            "--expect-sha256",
            &"a".repeat(64),
            "--replacement",
            "retryBudget",
            "--span",
            "40:5-40:9",
            "--expect-sha256",
            &"b".repeat(64),
            "--replacement",
            "four",
        ]))
        .expect("grouped spans parse");
        assert_eq!(cli.spans.len(), 2);
        assert_eq!(cli.spans[0]["startColumn"], json!(21));
        assert_eq!(cli.spans[0]["expectedSha256"], json!("a".repeat(64)));
        assert_eq!(cli.spans[1]["replacement"], json!("four"));
    }

    #[test]
    fn an_unaccompanied_digest_or_replacement_is_a_usage_error() {
        // The hazard: silently binding a digest to the wrong span would move
        // the guard off the span it was meant to protect.
        assert!(Cli::parse(&args(&[
            "--repo-path",
            ".",
            "--file",
            "a.rs",
            "--expect-sha256",
            &"a".repeat(64)
        ]))
        .is_err());
        assert!(Cli::parse(&args(&[
            "--repo-path",
            ".",
            "--file",
            "a.rs",
            "--span",
            "1:1-1:2",
            "--replacement",
            "x"
        ]))
        .is_err());
        assert!(Cli::parse(&args(&[
            "--repo-path",
            ".",
            "--file",
            "a.rs",
            "--span",
            "1:1-1:2",
            "--expect-sha256",
            &"a".repeat(64)
        ]))
        .is_err());
    }

    #[test]
    fn span_tokens_are_rejected_rather_than_guessed_at() {
        for token in ["12", "12:21", "12:21-12", "12:21-a:2", "-1:1-1:2"] {
            assert!(parse_span_token(token).is_err(), "{token} should not parse");
        }
        let span = parse_span_token("7:3-9:11").expect("well-formed span");
        assert_eq!(
            (
                span.start_line,
                span.start_column,
                span.end_line,
                span.end_column
            ),
            (7, 3, 9, 11)
        );
    }
}
