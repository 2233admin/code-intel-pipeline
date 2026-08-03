//! `edit.ast-grep-plan` — structural search and rewrite *planning* (#96 item
//! 2, charter gate G4 in #139).
//!
//! This capability never writes. What it publishes is an instruction: for
//! every match, the span it occupies in the current bytes, the sha256 of
//! exactly those bytes, the sha256 of the line they sit on, and the
//! replacement ast-grep computed. `apply.edits` is that instruction in the
//! coordinate system `span_patch` defines, which is what lets
//! `edit.ast-grep-apply` execute a rename without the model ever regenerating
//! a line — and lets it refuse, per match, when the bytes under a recorded
//! span have moved on.
//!
//! The line digest is not redundant with the span digest. A match is derived
//! from a *parse* of that line, and an identifier can change while the
//! planned bytes do not: renaming `commit` to `commit_now` leaves
//! `271:5-271:11` reading `commit`. Recording the line is what lets the apply
//! stage notice.
//!
//! The digest is recorded here rather than recomputed at apply time on
//! purpose. A digest computed at apply time would be a tautology: it would
//! only prove the file equals itself. Recorded at plan time, it is a claim
//! about the world that the apply stage can find false.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};

use super::span_patch::{self, LineIndex, SpanAddress};
use super::{
    publish_named, snapshot_adapter_error, tool_path, AdapterArtifact, AdapterError, AdapterOutput,
};
use crate::adapter_contract::AdapterDomainVerdict;
use crate::artifact_ref::VerifiedArtifact;
use crate::capability::sha256_hex;
use crate::snapshot;

const MAX_PATTERN_BYTES: usize = 16 * 1024;
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_PATHS: usize = 64;
/// Bounds on what may become an *applicable* plan. A preview may report any
/// number of matches of any size; an instruction that rewrites bytes stays
/// small enough to read, to hash, and to refuse as a unit.
const MAX_PLAN_EDITS: usize = 256;
const MAX_PLAN_SPAN_BYTES: usize = 4096;

pub(crate) fn execute(
    request: &Value,
    verified_inputs: &[VerifiedArtifact],
    out: &Path,
) -> Result<AdapterOutput, AdapterError> {
    if !verified_inputs.is_empty() {
        return Err(AdapterError::Contract(
            "edit.ast-grep-plan does not accept input artifacts".into(),
        ));
    }
    let options = request["options"]
        .as_object()
        .ok_or_else(|| AdapterError::InvalidOptions("options must be an object".into()))?;
    if options.keys().any(|key| {
        !matches!(
            key.as_str(),
            "repoPath" | "language" | "pattern" | "rewrite" | "paths"
        )
    }) {
        return Err(AdapterError::InvalidOptions(
            "edit.ast-grep-plan accepts only repoPath/language/pattern/rewrite/paths".into(),
        ));
    }
    let repo = required_string(options.get("repoPath"), "options.repoPath")?;
    let repo = Path::new(repo);
    if !repo.is_dir() {
        return Err(AdapterError::InvalidOptions(format!(
            "repoPath is not a directory: {}",
            repo.display()
        )));
    }
    let language = required_string(options.get("language"), "options.language")?;
    if language.len() > 64 {
        return Err(AdapterError::InvalidOptions(
            "options.language exceeds 64 bytes".into(),
        ));
    }
    let pattern = bounded_string(options.get("pattern"), "options.pattern")?;
    let rewrite = options
        .get("rewrite")
        .map(|value| bounded_string(Some(value), "options.rewrite"))
        .transpose()?;
    let paths = requested_paths(options.get("paths"))?;
    let canonical_repo = fs::canonicalize(repo)
        .map_err(|error| AdapterError::Io(format!("resolve repoPath: {error}")))?;
    let snapshot_scopes = request["snapshot"]["scope"]
        .as_array()
        .expect("validated snapshot scope")
        .iter()
        .map(|value| {
            span_patch::normalize_relative(
                value.as_str().expect("validated snapshot scope item"),
                false,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let paths = paths
        .into_iter()
        .map(|path| validate_path(repo, &canonical_repo, &snapshot_scopes, path))
        .collect::<Result<Vec<_>, _>>()?;

    let lease =
        snapshot::begin_consumption(repo, &request["snapshot"]).map_err(snapshot_adapter_error)?;
    let version = ast_grep_version()?;
    let mut command = ast_grep_command();
    command
        .args(["run", "--pattern"])
        .arg(pattern)
        .args(["--lang", language]);
    if let Some(rewrite) = rewrite {
        command.args(["--rewrite", rewrite]);
    }
    command.args(["--json=compact", "--threads", "0", "--"]);
    command.args(&paths).current_dir(repo);
    let output = command
        .output()
        .map_err(|error| AdapterError::Unavailable(format!("start ast-grep: {error}")))?;
    if !output.status.success() {
        return Err(AdapterError::Internal(format!(
            "ast-grep failed: {}",
            bounded_diagnostic(&output.stderr)
        )));
    }
    if output.stdout.len() > MAX_OUTPUT_BYTES {
        return Err(AdapterError::Contract(format!(
            "ast-grep output exceeds {MAX_OUTPUT_BYTES} bytes; narrow options.paths or pattern"
        )));
    }
    let mut matches: Vec<Value> = serde_json::from_slice(&output.stdout).map_err(|error| {
        AdapterError::Internal(format!("ast-grep emitted invalid JSON: {error}"))
    })?;
    let mut files = BTreeSet::new();
    for item in &mut matches {
        let file = item
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| AdapterError::Internal("ast-grep match has no file path".into()))?;
        let file = normalize_match_file(repo, &canonical_repo, file)?;
        item["file"] = Value::String(file.clone());
        files.insert(file);
    }
    let apply = apply_block(&canonical_repo, &matches, rewrite.is_some())?;
    lease.verify_after(repo).map_err(snapshot_adapter_error)?;

    let artifact = json!({
        "schema": "code-intel-structured-edit-plan.v1",
        "capability": "edit.ast-grep-plan",
        "snapshotIdentity": request["snapshot"]["identity"],
        "tool": {
            "name": "ast-grep",
            "version": version,
            "threads": "auto"
        },
        "query": {
            "language": language,
            "pattern": pattern,
            "rewrite": rewrite,
            "paths": paths
        },
        "summary": {
            "matches": matches.len(),
            "files": files.len(),
            "hasRewrite": rewrite.is_some(),
            "applicableEdits": apply["edits"].as_array().map_or(0, Vec::len)
        },
        "matches": matches,
        "apply": apply,
        "authority": {
            "mode": "preview_only",
            "repositoryMutation": false
        }
    });
    let bytes = serde_json::to_vec(&artifact)
        .map_err(|error| AdapterError::Internal(format!("serialize edit plan: {error}")))?;
    publish_named(out, "structured-edit-plan.json", &bytes, |_| Ok(()))?;
    Ok(AdapterOutput {
        artifacts: vec![AdapterArtifact {
            artifact_schema: "code-intel-structured-edit-plan.v1".into(),
            artifact_type: "edit.structured-plan".into(),
            relative_path: "structured-edit-plan.json".into(),
            bytes,
        }],
        observed_effects: vec![
            "repo_read".into(),
            "local_write".into(),
            "process_spawn".into(),
        ],
        domain_verdict: AdapterDomainVerdict::Pass,
        domain_failure: None,
    })
}

fn required_string<'a>(value: Option<&'a Value>, name: &str) -> Result<&'a str, AdapterError> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AdapterError::InvalidOptions(format!("{name} must be non-empty")))
}

fn bounded_string<'a>(value: Option<&'a Value>, name: &str) -> Result<&'a str, AdapterError> {
    let value = required_string(value, name)?;
    if value.len() > MAX_PATTERN_BYTES {
        Err(AdapterError::InvalidOptions(format!(
            "{name} exceeds {MAX_PATTERN_BYTES} bytes"
        )))
    } else {
        Ok(value)
    }
}

fn requested_paths(value: Option<&Value>) -> Result<Vec<&str>, AdapterError> {
    match value {
        None => Ok(vec!["."]),
        Some(value) => {
            let paths = value.as_array().ok_or_else(|| {
                AdapterError::InvalidOptions("options.paths must be an array".into())
            })?;
            if paths.is_empty() || paths.len() > MAX_PATHS {
                return Err(AdapterError::InvalidOptions(format!(
                    "options.paths must contain 1..={MAX_PATHS} entries"
                )));
            }
            paths
                .iter()
                .map(|value| required_string(Some(value), "options.paths[]"))
                .collect()
        }
    }
}

/// One match, reduced to the instruction the apply stage executes.
struct PlannedEdit {
    index: usize,
    file: String,
    start: usize,
    end: usize,
    address: SpanAddress,
    sha256: String,
    line_sha256: String,
    text: String,
    replacement: String,
}

/// Turn ast-grep's matches into instructions, or say plainly why they cannot
/// become one.
///
/// `applicable:false` is not a failure — a plan without a rewrite is a
/// legitimate search preview. It is a refusal to hand the apply stage
/// something it would have to guess about, and the reason travels with it so
/// the caller does not have to re-derive it.
fn apply_block(
    canonical_repo: &Path,
    matches: &[Value],
    has_rewrite: bool,
) -> Result<Value, AdapterError> {
    if !has_rewrite {
        return Ok(inapplicable(
            "plan carries no rewrite, so it is a search preview rather than an instruction",
            Vec::new(),
        ));
    }
    if matches.is_empty() {
        return Ok(inapplicable("plan matched nothing", Vec::new()));
    }
    if matches.len() > MAX_PLAN_EDITS {
        return Ok(inapplicable(
            &format!(
                "plan has {} matches; an applicable plan carries at most {MAX_PLAN_EDITS}",
                matches.len()
            ),
            Vec::new(),
        ));
    }
    let mut sources: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut edits = Vec::new();
    let mut exclusion: Option<String> = None;
    for (index, item) in matches.iter().enumerate() {
        let file = item["file"]
            .as_str()
            .expect("match file normalized above")
            .to_string();
        if !sources.contains_key(&file) {
            let bytes = fs::read(canonical_repo.join(&file))
                .map_err(|error| AdapterError::Io(format!("read {file}: {error}")))?;
            sources.insert(file.clone(), bytes);
        }
        let source = &sources[&file];
        match planned_edit(index, &file, source, item) {
            Ok(edit) => edits.push(edit),
            Err(reason) => {
                exclusion.get_or_insert(format!("match {index} in {file}: {reason}"));
            }
        }
    }
    edits.sort_by(|left, right| {
        left.file
            .cmp(&right.file)
            .then_with(|| left.start.cmp(&right.start))
    });
    for pair in edits.windows(2) {
        if pair[0].file == pair[1].file && pair[1].start < pair[0].end {
            exclusion.get_or_insert(format!(
                "matches {} and {} overlap in {}",
                pair[0].index, pair[1].index, pair[0].file
            ));
        }
    }
    let rows = edits
        .iter()
        .map(|edit| {
            json!({
                "match": edit.index,
                "file": edit.file,
                "span": edit.address.as_json(),
                "byteRange": {"start": edit.start, "end": edit.end},
                "sha256": edit.sha256,
                "lineSha256": edit.line_sha256,
                "bytes": edit.end - edit.start,
                "text": edit.text,
                "replacement": edit.replacement,
            })
        })
        .collect::<Vec<_>>();
    Ok(match exclusion {
        Some(reason) => inapplicable(&reason, rows),
        None => json!({
            "capability": "edit.ast-grep-apply",
            "applicable": true,
            "reason": Value::Null,
            "edits": rows,
        }),
    })
}

fn inapplicable(reason: &str, edits: Vec<Value>) -> Value {
    json!({
        "capability": "edit.ast-grep-apply",
        "applicable": false,
        "reason": reason,
        "edits": edits,
    })
}

fn planned_edit(
    index: usize,
    file: &str,
    source: &[u8],
    item: &Value,
) -> Result<PlannedEdit, String> {
    let (start, end) = replacement_range(item)?;
    if end > source.len() {
        return Err(format!(
            "byte range {start}..{end} is beyond the {} byte file",
            source.len()
        ));
    }
    if end - start > MAX_PLAN_SPAN_BYTES {
        return Err(format!(
            "match spans {} bytes; an applicable edit spans at most {MAX_PLAN_SPAN_BYTES}",
            end - start
        ));
    }
    let replacement = item["replacement"]
        .as_str()
        .ok_or("ast-grep reported no replacement for this match")?;
    if replacement.len() > MAX_PLAN_SPAN_BYTES {
        return Err(format!(
            "replacement is {} bytes; an applicable edit replaces with at most {MAX_PLAN_SPAN_BYTES}",
            replacement.len()
        ));
    }
    let index_of_lines = LineIndex::build(source);
    let address = index_of_lines.address(start, end)?;
    let (line_start, line_end) = index_of_lines.line_span(&address)?;
    let text = std::str::from_utf8(&source[start..end])
        .map_err(|error| format!("matched bytes are not UTF-8: {error}"))?;
    Ok(PlannedEdit {
        index,
        file: file.to_string(),
        start,
        end,
        address,
        sha256: sha256_hex(&source[start..end]),
        line_sha256: sha256_hex(&source[line_start..line_end]),
        text: text.to_string(),
        replacement: replacement.to_string(),
    })
}

/// The byte range ast-grep would actually replace. It reports
/// `replacementOffsets` alongside the match range; taking that when present
/// keeps the edit as narrow as the tool made it, which is the whole point of
/// G4 — a rewrite that only alters part of a match must not rewrite the rest.
fn replacement_range(item: &Value) -> Result<(usize, usize), String> {
    let offsets = if item["replacementOffsets"].is_object() {
        &item["replacementOffsets"]
    } else {
        &item["range"]["byteOffset"]
    };
    let bound = |key: &str| -> Result<usize, String> {
        offsets[key]
            .as_u64()
            .filter(|value| *value <= u32::MAX as u64)
            .map(|value| value as usize)
            .ok_or_else(|| format!("ast-grep match has no usable byte offset {key}"))
    };
    let (start, end) = (bound("start")?, bound("end")?);
    if start >= end {
        return Err(format!("byte range {start}..{end} is empty or inverted"));
    }
    Ok((start, end))
}

fn validate_path(
    repo: &Path,
    canonical_repo: &Path,
    snapshot_scopes: &[String],
    path: &str,
) -> Result<String, AdapterError> {
    let normalized = span_patch::normalize_relative(path, true)?;
    if !snapshot_scopes
        .iter()
        .any(|scope| span_patch::within_scope(&normalized, scope))
    {
        return Err(AdapterError::Contract(format!(
            "edit path is outside the requested snapshot scope: {path}"
        )));
    }
    let full = fs::canonicalize(repo.join(Path::new(&normalized))).map_err(|error| {
        AdapterError::InvalidOptions(format!("resolve edit path {path}: {error}"))
    })?;
    if !full.starts_with(canonical_repo) {
        return Err(AdapterError::Contract(format!(
            "edit path escapes repository: {path}"
        )));
    }
    Ok(normalized)
}

fn normalize_match_file(
    repo: &Path,
    canonical_repo: &Path,
    file: &str,
) -> Result<String, AdapterError> {
    let file = Path::new(file);
    let full = if file.is_absolute() {
        file.to_path_buf()
    } else {
        repo.join(file)
    };
    let full = fs::canonicalize(&full)
        .map_err(|error| AdapterError::Internal(format!("resolve ast-grep match: {error}")))?;
    let relative = full.strip_prefix(canonical_repo).map_err(|_| {
        AdapterError::Contract(format!(
            "ast-grep returned a match outside the repository: {}",
            full.display()
        ))
    })?;
    span_patch::normalize_relative(&relative.to_string_lossy(), true)
}

/// A command that launches `ast-grep` by absolute path, resolved through
/// `tool_path` like every `rg`/`git` call site: `execute` runs the child in
/// the scanned repository, so a bare name must never reach `Command::new`.
/// npm-style installs ship a `.cmd` shim on Windows, which `CreateProcess`
/// cannot start directly, so those are wrapped in `cmd.exe` the same way
/// `builtin_provider_evidence::external_command` wraps them.
fn ast_grep_command() -> Command {
    let path = tool_path::resolve("ast-grep");
    #[cfg(windows)]
    if path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "cmd" | "bat"))
    {
        let mut command = Command::new("cmd.exe");
        command.args(["/d", "/c"]).arg(path);
        return command;
    }
    Command::new(path)
}

fn ast_grep_version() -> Result<String, AdapterError> {
    let output = ast_grep_command()
        .arg("--version")
        .output()
        .map_err(|error| AdapterError::Unavailable(format!("start ast-grep: {error}")))?;
    if !output.status.success() {
        return Err(AdapterError::Unavailable(format!(
            "ast-grep --version failed: {}",
            bounded_diagnostic(&output.stderr)
        )));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|error| {
            AdapterError::Unavailable(format!("ast-grep version is not UTF-8: {error}"))
        })
}

fn bounded_diagnostic(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(4096)])
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn match_at(file: &str, start: usize, end: usize, replacement: &str) -> Value {
        json!({
            "file": file,
            "range": {"byteOffset": {"start": start, "end": end}},
            "replacement": replacement,
        })
    }

    #[test]
    fn registry_digest_and_path_guards_are_bound_to_this_adapter() {
        let registry: Value =
            serde_json::from_slice(include_bytes!("../../../orchestration/integrations.json"))
                .unwrap();
        let integration = registry["integrations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "edit.ast-grep-plan")
            .unwrap();
        assert_eq!(
            integration["capabilityDeclaration"]["implementation"]["toolchainDigests"],
            json!([
                sha256_hex(include_bytes!("structured_edit.rs")),
                sha256_hex(include_bytes!("span_patch.rs"))
            ])
        );
        // The plan stage still declares no write authority. What it gained is
        // the ability to *describe* one.
        assert_eq!(
            integration["capabilityDeclaration"]["allowedEffects"],
            json!(["repo_read", "local_write", "process_spawn"])
        );
    }

    /// A rewrite that only changes part of a match must plan only that part —
    /// this is G4's exit condition expressed at the planning stage.
    #[test]
    fn a_narrower_replacement_range_wins_over_the_match_range() {
        let mut item = match_at("a.rs", 0, 20, "x");
        item["replacementOffsets"] = json!({"start": 4, "end": 9});
        assert_eq!(replacement_range(&item).unwrap(), (4, 9));
        item["replacementOffsets"] = Value::Null;
        assert_eq!(replacement_range(&item).unwrap(), (0, 20));
    }

    /// Two matches that overlap have no single well-defined result, so the
    /// plan says so instead of handing the apply stage a coin flip.
    #[test]
    fn overlapping_matches_make_the_whole_plan_inapplicable() {
        let source = "let alpha = alpha_beta;\n";
        let directory = std::env::temp_dir().join(format!(
            "code-intel-plan-overlap-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("a.rs"), source).unwrap();

        let disjoint = apply_block(
            &directory,
            &[
                match_at("a.rs", 4, 9, "beta"),
                match_at("a.rs", 12, 22, "x"),
            ],
            true,
        )
        .unwrap();
        assert_eq!(disjoint["applicable"], json!(true));
        assert_eq!(disjoint["edits"][0]["sha256"], json!(sha256_hex(b"alpha")));
        assert_eq!(disjoint["edits"][0]["text"], json!("alpha"));
        assert_eq!(disjoint["edits"][0]["span"]["startColumn"], json!(5));

        let overlapping = apply_block(
            &directory,
            &[match_at("a.rs", 4, 9, "beta"), match_at("a.rs", 6, 12, "x")],
            true,
        )
        .unwrap();
        assert_eq!(overlapping["applicable"], json!(false));
        assert!(overlapping["reason"].as_str().unwrap().contains("overlap"));

        // A search preview is legitimately inapplicable rather than broken.
        let preview = apply_block(&directory, &[match_at("a.rs", 4, 9, "beta")], false).unwrap();
        assert_eq!(preview["applicable"], json!(false));
        assert_eq!(preview["edits"], json!([]));
        let _ = fs::remove_dir_all(&directory);
    }
}
