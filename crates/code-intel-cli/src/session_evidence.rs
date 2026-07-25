use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::capability::sha256_hex;

const MAX_TRACE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_HOTSPOT_BYTES: u64 = 64 * 1024 * 1024;
const MINDWALK_COMPATIBILITY_REVISION: &str = "e208b6b8504138843f671e031f28129b66003a67";

#[derive(Debug)]
enum AdapterError {
    Usage(String),
    Contract(String),
    Io(String),
}

impl AdapterError {
    fn exit_code(&self) -> i32 {
        match self {
            Self::Usage(_) => 64,
            Self::Contract(_) => 65,
            Self::Io(_) => 74,
        }
    }

    fn message(&self) -> &str {
        match self {
            Self::Usage(message) | Self::Contract(message) | Self::Io(message) => message,
        }
    }
}

struct Cli {
    repo: PathBuf,
    trace: PathBuf,
    hotspots: Option<PathBuf>,
    out: Option<PathBuf>,
    working_tree_policy: String,
}

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let cli = match parse_cli(raw) {
        Ok(cli) => cli,
        Err(error) => {
            eprintln!("{}", error.message());
            return error.exit_code();
        }
    };
    let artifact = match adapt(&cli) {
        Ok(artifact) => artifact,
        Err(error) => {
            eprintln!("{}", error.message());
            return error.exit_code();
        }
    };
    let bytes = serde_json::to_vec_pretty(&artifact).expect("session evidence serializes");
    if let Some(path) = cli.out.as_deref() {
        if let Err(error) = write_new_artifact(path, &bytes) {
            eprintln!("{}", error.message());
            return error.exit_code();
        }
        let receipt = json!({
            "schema":"code-intel-session-adapter-result.v1",
            "status":"completed",
            "artifactSha256":sha256_hex(&bytes),
            "summary":artifact["summary"]
        });
        println!("{}", serde_json::to_string(&receipt).unwrap());
    } else {
        println!("{}", String::from_utf8(bytes).expect("JSON is UTF-8"));
    }
    0
}

fn parse_cli(raw: &[String]) -> Result<Cli, AdapterError> {
    let mut repo = None;
    let mut trace = None;
    let mut hotspots = None;
    let mut out = None;
    let mut working_tree_policy = "explicit_overlay".to_string();
    let mut index = 0;
    while index < raw.len() {
        let flag = raw[index].as_str();
        if !matches!(
            flag,
            "--repo" | "--trace" | "--hotspots" | "--out" | "--working-tree-policy"
        ) {
            return Err(AdapterError::Usage(format!(
                "unknown session adapter argument: {flag}"
            )));
        }
        let value = raw
            .get(index + 1)
            .filter(|value| !value.is_empty() && !value.starts_with("--"))
            .ok_or_else(|| AdapterError::Usage(format!("{flag} requires exactly one value")))?;
        match flag {
            "--repo" if repo.replace(PathBuf::from(value)).is_some() => {
                return Err(AdapterError::Usage("duplicate --repo".into()))
            }
            "--trace" if trace.replace(PathBuf::from(value)).is_some() => {
                return Err(AdapterError::Usage("duplicate --trace".into()))
            }
            "--hotspots" if hotspots.replace(PathBuf::from(value)).is_some() => {
                return Err(AdapterError::Usage("duplicate --hotspots".into()))
            }
            "--out" if out.replace(PathBuf::from(value)).is_some() => {
                return Err(AdapterError::Usage("duplicate --out".into()))
            }
            "--working-tree-policy" => {
                if !matches!(value.as_str(), "head_only" | "explicit_overlay") {
                    return Err(AdapterError::Usage(
                        "--working-tree-policy must be head_only or explicit_overlay".into(),
                    ));
                }
                working_tree_policy = value.clone();
            }
            _ => {}
        }
        index += 2;
    }
    let repo = repo.ok_or_else(|| AdapterError::Usage("--repo is required".into()))?;
    if !repo.is_dir() {
        return Err(AdapterError::Usage(format!(
            "repository path is not a directory: {}",
            repo.display()
        )));
    }
    Ok(Cli {
        repo,
        trace: trace.ok_or_else(|| AdapterError::Usage("--trace is required".into()))?,
        hotspots,
        out,
        working_tree_policy,
    })
}

fn adapt(cli: &Cli) -> Result<Value, AdapterError> {
    let trace_bytes = read_bounded_regular(&cli.trace, MAX_TRACE_BYTES, "session trace")?;
    let trace: Value = serde_json::from_slice(&trace_bytes)
        .map_err(|_| AdapterError::Contract("session trace is not valid JSON".into()))?;
    validate_trace(&trace)?;

    let repo = fs::canonicalize(&cli.repo)
        .map_err(|error| AdapterError::Io(format!("cannot resolve repository root: {error}")))?;
    let snapshot =
        crate::snapshot::build_for_dag(&repo, &cli.working_tree_policy, &[".".to_string()])
            .map_err(AdapterError::Contract)?;
    let hotspot_index = match cli.hotspots.as_deref() {
        Some(path) => {
            let bytes = read_bounded_regular(path, MAX_HOTSPOT_BYTES, "Sentrux enrichment")?;
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
                AdapterError::Contract("Sentrux enrichment is not valid JSON".into())
            })?;
            build_hotspot_index(&value)?
        }
        None => BTreeMap::new(),
    };

    let session = &trace["session"];
    let session_cwd = session["cwd"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.clone());
    let session_cwd = fs::canonicalize(&session_cwd).unwrap_or(session_cwd);
    let observability = &trace["stats"]["observability"];
    let target_grade = grade(observability["reads"].as_str());
    let error_grade = grade(observability["errors"].as_str());
    let raw_events = trace["events"].as_array().unwrap();
    let last_verify = raw_events
        .iter()
        .filter(|event| event["action"] == "verify")
        .filter_map(|event| event["seq"].as_u64())
        .max();

    let mut events = Vec::with_capacity(raw_events.len());
    let mut signals = Vec::new();
    let mut seen_seq = BTreeSet::new();
    let mut target_count = 0_u64;
    let mut matched_target_count = 0_u64;
    let mut unmatched_target_count = 0_u64;
    let mut unsafe_target_count = 0_u64;
    let mut structural_attention_touches = 0_u64;
    let mut edit_events = 0_u64;
    let mut verify_events = 0_u64;
    let mut error_events = 0_u64;

    for raw_event in raw_events {
        let seq = raw_event["seq"].as_u64().unwrap();
        if !seen_seq.insert(seq) {
            return Err(AdapterError::Contract(
                "session trace event sequence values must be unique".into(),
            ));
        }
        let action = normalized_action(raw_event["action"].as_str());
        let is_error = raw_event["isError"].as_bool().unwrap();
        let raw_targets = raw_event["targets"].as_array().unwrap();
        let mut targets = Vec::new();
        let event_has_edit = action == "edit"
            || raw_targets
                .iter()
                .any(|target| normalized_touch(target["touch"].as_str()) == "edit");
        for raw_target in raw_targets {
            target_count += 1;
            let raw_path = raw_target["path"].as_str().unwrap();
            let Some(path) = normalize_target(&repo, &session_cwd, raw_path) else {
                unsafe_target_count += 1;
                continue;
            };
            let touch = normalized_touch(raw_target["touch"].as_str());
            let structural = if let Some(hotspot) = hotspot_index.get(&path) {
                matched_target_count += 1;
                let attention = structural_attention(hotspot);
                if attention {
                    structural_attention_touches += 1;
                    if event_has_edit {
                        signals.push(signal("structural_attention_edit", seq, &path, "review"));
                    }
                    if is_error {
                        signals.push(signal(
                            "error_on_structural_attention",
                            seq,
                            &path,
                            "review",
                        ));
                    }
                    if event_has_edit && last_verify.is_none_or(|verify| seq > verify) {
                        signals.push(signal(
                            "unverified_structural_attention_edit",
                            seq,
                            &path,
                            "review",
                        ));
                    }
                }
                json!({
                    "status":"matched",
                    "attention":attention,
                    "maxComplexity":u64_any(hotspot, &["maxComplexity", "max_complexity"]),
                    "avgComplexity":f64_any(hotspot, &["avgComplexity", "avg_complexity"]),
                    "loc":hotspot["loc"].as_u64().unwrap_or(0),
                    "gitChurn":hotspot["git"]["churn"].as_u64().unwrap_or(0),
                    "dirty":hotspot["git"]["dirty"].as_bool().unwrap_or(false)
                })
            } else {
                unmatched_target_count += 1;
                json!({"status":"unknown"})
            };
            targets.push(json!({
                "path":path,
                "touch":touch,
                "observability":target_grade,
                "structural":structural
            }));
        }
        targets.sort_by(|left, right| {
            (left["path"].as_str(), left["touch"].as_str())
                .cmp(&(right["path"].as_str(), right["touch"].as_str()))
        });
        targets.dedup_by(|left, right| {
            left["path"] == right["path"] && left["touch"] == right["touch"]
        });
        unsafe_target_count += raw_event["outside"]
            .as_array()
            .map(|outside| outside.len() as u64)
            .unwrap_or(0);
        edit_events += u64::from(event_has_edit);
        verify_events += u64::from(action == "verify");
        error_events += u64::from(is_error);
        events.push(json!({
            "seq":seq,
            "action":action,
            "toolFamily":tool_family(raw_event["tool"].as_str()),
            "isError":is_error,
            "targets":targets
        }));
    }
    events.sort_by_key(|event| event["seq"].as_u64().unwrap());
    signals.sort_by(|left, right| {
        (
            left["eventSeq"].as_u64(),
            left["path"].as_str(),
            left["kind"].as_str(),
        )
            .cmp(&(
                right["eventSeq"].as_u64(),
                right["path"].as_str(),
                right["kind"].as_str(),
            ))
    });
    signals.dedup();

    // Mindwalk v1 exposes tool activity, but edit and verification intent remain
    // heuristic. Keep the aggregate status honest even when every target joins.
    let status = "partial";
    let source_session_id = session["id"].as_str().unwrap();
    let harness = coarse_harness_family(session["harness"].as_str().unwrap());
    let artifact = json!({
        "schema":"code-intel-session-evidence.v1",
        "status":status,
        "reviewAuthority":"advisory_only",
        "snapshot":snapshot["snapshot"],
        "source":{
            "provider":"mindwalk",
            "traceSchema":"mindwalk-trace.v1",
            "compatibilityRevision":MINDWALK_COMPATIBILITY_REVISION,
            "license":"MIT",
            "copiedSource":false,
            "traceSha256":sha256_hex(&trace_bytes),
            "sessionDigest":sha256_hex(source_session_id.as_bytes()),
            "harness":harness
        },
        "implementation":{
            "id":"code-intel.session-evidence.rust",
            "version":"1"
        },
        "privacy":{
            "rawTracePersisted":false,
            "userMessageMarksConsumed":false,
            "eventSummariesConsumed":false,
            "absolutePathsEmitted":false
        },
        "observability":{
            "events":"exact",
            "targets":target_grade,
            "errors":error_grade,
            "edits":"estimated",
            "verification":"estimated"
        },
        "summary":{
            "events":events.len(),
            "targetEvents":events.iter().filter(|event| event["targets"].as_array().is_some_and(|targets| !targets.is_empty())).count(),
            "targets":target_count,
            "matchedTargets":matched_target_count,
            "unmatchedTargets":unmatched_target_count,
            "unsafeOrOutsideTargets":unsafe_target_count,
            "structuralAttentionTouches":structural_attention_touches,
            "editEvents":edit_events,
            "verifyEvents":verify_events,
            "errorEvents":error_events,
            "signals":signals.len()
        },
        "events":events,
        "signals":signals
    });
    validate_artifact_value(&artifact).map_err(AdapterError::Contract)?;
    Ok(artifact)
}

pub(crate) fn validate_artifact_value(value: &Value) -> Result<(), String> {
    exact_keys(
        value,
        &[
            "schema",
            "status",
            "reviewAuthority",
            "snapshot",
            "source",
            "implementation",
            "privacy",
            "observability",
            "summary",
            "events",
            "signals",
        ],
        "session evidence",
    )?;
    if value["schema"] != "code-intel-session-evidence.v1"
        || !matches!(value["status"].as_str(), Some("complete" | "partial"))
        || value["reviewAuthority"] != "advisory_only"
    {
        return Err("session evidence authority or top-level identity is invalid".into());
    }
    validate_snapshot(&value["snapshot"])?;
    validate_source(&value["source"])?;
    exact_keys(
        &value["implementation"],
        &["id", "version"],
        "implementation",
    )?;
    if value["implementation"]["id"] != "code-intel.session-evidence.rust"
        || value["implementation"]["version"] != "1"
    {
        return Err("session evidence implementation identity is invalid".into());
    }
    exact_keys(
        &value["privacy"],
        &[
            "rawTracePersisted",
            "userMessageMarksConsumed",
            "eventSummariesConsumed",
            "absolutePathsEmitted",
        ],
        "privacy",
    )?;
    if [
        "rawTracePersisted",
        "userMessageMarksConsumed",
        "eventSummariesConsumed",
        "absolutePathsEmitted",
    ]
    .iter()
    .any(|field| value["privacy"][field] != false)
    {
        return Err("session evidence privacy boundary is invalid".into());
    }
    exact_keys(
        &value["observability"],
        &["events", "targets", "errors", "edits", "verification"],
        "observability",
    )?;
    if ["events", "targets", "errors", "edits", "verification"]
        .iter()
        .any(|field| !is_grade(value["observability"][field].as_str()))
    {
        return Err("session evidence observability grade is invalid".into());
    }
    validate_session_body(value)
}

fn validate_snapshot(value: &Value) -> Result<(), String> {
    exact_keys(
        value,
        &[
            "identity",
            "repoIdentity",
            "head",
            "workingTreePolicy",
            "scope",
            "inputDigest",
        ],
        "snapshot",
    )?;
    let scopes = value["scope"]
        .as_array()
        .filter(|items| !items.is_empty())
        .ok_or_else(|| "session evidence snapshot scope is invalid".to_string())?;
    if !value["identity"].as_str().is_some_and(valid_digest)
        || !value["inputDigest"].as_str().is_some_and(valid_digest)
        || value["repoIdentity"].as_str().is_none_or(str::is_empty)
        || value["head"].as_str().is_none_or(str::is_empty)
        || !matches!(
            value["workingTreePolicy"].as_str(),
            Some("head_only" | "explicit_overlay")
        )
        || scopes
            .iter()
            .any(|scope| scope.as_str().is_none_or(str::is_empty))
    {
        return Err("session evidence snapshot contract is invalid".into());
    }
    Ok(())
}

fn validate_source(value: &Value) -> Result<(), String> {
    exact_keys(
        value,
        &[
            "provider",
            "traceSchema",
            "compatibilityRevision",
            "license",
            "copiedSource",
            "traceSha256",
            "sessionDigest",
            "harness",
        ],
        "source",
    )?;
    if value["provider"] != "mindwalk"
        || value["traceSchema"] != "mindwalk-trace.v1"
        || value["license"] != "MIT"
        || value["copiedSource"] != false
        || !value["compatibilityRevision"]
            .as_str()
            .is_some_and(|revision| revision.len() == 40 && revision.bytes().all(is_lower_hex))
        || !value["traceSha256"].as_str().is_some_and(valid_digest)
        || !value["sessionDigest"].as_str().is_some_and(valid_digest)
        || value["harness"].as_str().is_none_or(str::is_empty)
    {
        return Err("session evidence source contract is invalid".into());
    }
    Ok(())
}

fn validate_session_body(value: &Value) -> Result<(), String> {
    const SUMMARY_FIELDS: [&str; 11] = [
        "events",
        "targetEvents",
        "targets",
        "matchedTargets",
        "unmatchedTargets",
        "unsafeOrOutsideTargets",
        "structuralAttentionTouches",
        "editEvents",
        "verifyEvents",
        "errorEvents",
        "signals",
    ];
    exact_keys(&value["summary"], &SUMMARY_FIELDS, "summary")?;
    if SUMMARY_FIELDS
        .iter()
        .any(|field| value["summary"][field].as_u64().is_none())
    {
        return Err("session evidence summary contains a non-counter".into());
    }
    let events = value["events"]
        .as_array()
        .filter(|events| !events.is_empty())
        .ok_or_else(|| "session evidence requires events".to_string())?;
    let signals = value["signals"]
        .as_array()
        .ok_or_else(|| "session evidence signals must be an array".to_string())?;
    let mut sequences = BTreeSet::new();
    let mut target_events = 0_u64;
    let mut targets = 0_u64;
    let mut matched = 0_u64;
    let mut attention = 0_u64;
    let mut edits = 0_u64;
    let mut verifies = 0_u64;
    let mut errors = 0_u64;
    for event in events {
        validate_event(event, &mut sequences)?;
        let event_targets = event["targets"].as_array().expect("validated targets");
        target_events += u64::from(!event_targets.is_empty());
        targets += event_targets.len() as u64;
        for target in event_targets {
            if target["structural"]["status"] == "matched" {
                matched += 1;
                attention += u64::from(target["structural"]["attention"] == true);
            }
        }
        edits += u64::from(
            event["action"] == "edit"
                || event_targets.iter().any(|target| target["touch"] == "edit"),
        );
        verifies += u64::from(event["action"] == "verify");
        errors += u64::from(event["isError"] == true);
    }
    for signal in signals {
        validate_signal(signal, &sequences)?;
    }
    let summary = &value["summary"];
    let exact = [
        ("events", events.len() as u64),
        ("targetEvents", target_events),
        ("matchedTargets", matched),
        ("structuralAttentionTouches", attention),
        ("editEvents", edits),
        ("verifyEvents", verifies),
        ("errorEvents", errors),
        ("signals", signals.len() as u64),
    ];
    if exact
        .iter()
        .any(|(field, expected)| summary[field].as_u64() != Some(*expected))
        || summary["matchedTargets"].as_u64().unwrap()
            + summary["unmatchedTargets"].as_u64().unwrap()
            > summary["targets"].as_u64().unwrap()
        || summary["targets"].as_u64().unwrap() < targets
    {
        return Err("session evidence summary does not match normalized events".into());
    }
    Ok(())
}

fn validate_event(event: &Value, sequences: &mut BTreeSet<u64>) -> Result<(), String> {
    exact_keys(
        event,
        &["seq", "action", "toolFamily", "isError", "targets"],
        "event",
    )?;
    let seq = event["seq"]
        .as_u64()
        .filter(|seq| sequences.insert(*seq))
        .ok_or_else(|| "session evidence event sequence is invalid or duplicated".to_string())?;
    let _ = seq;
    if !matches!(
        event["action"].as_str(),
        Some("search" | "read" | "edit" | "exec" | "verify" | "other")
    ) || !matches!(
        event["toolFamily"].as_str(),
        Some("edit" | "shell" | "read" | "search" | "orchestration" | "other")
    ) || !event["isError"].is_boolean()
    {
        return Err("session evidence event classification is invalid".into());
    }
    for target in event["targets"]
        .as_array()
        .ok_or_else(|| "session evidence event targets must be an array".to_string())?
    {
        validate_target(target)?;
    }
    Ok(())
}

fn validate_target(target: &Value) -> Result<(), String> {
    exact_keys(
        target,
        &["path", "touch", "observability", "structural"],
        "target",
    )?;
    let path = target["path"]
        .as_str()
        .filter(|path| normalize_artifact_path(path).as_deref() == Some(*path))
        .ok_or_else(|| {
            "session evidence target path is not normalized repository-relative syntax".to_string()
        })?;
    let _ = path;
    if !matches!(target["touch"].as_str(), Some("hit" | "read" | "edit"))
        || !is_grade(target["observability"].as_str())
    {
        return Err("session evidence target classification is invalid".into());
    }
    let structural = &target["structural"];
    match structural["status"].as_str() {
        Some("unknown") => exact_keys(structural, &["status"], "unknown structural target"),
        Some("matched") => {
            exact_keys(
                structural,
                &[
                    "status",
                    "attention",
                    "maxComplexity",
                    "avgComplexity",
                    "loc",
                    "gitChurn",
                    "dirty",
                ],
                "matched structural target",
            )?;
            if !structural["attention"].is_boolean()
                || structural["maxComplexity"].as_u64().is_none()
                || structural["avgComplexity"].as_f64().is_none()
                || structural["loc"].as_u64().is_none()
                || structural["gitChurn"].as_u64().is_none()
                || !structural["dirty"].is_boolean()
            {
                return Err("session evidence structural measurement is invalid".into());
            }
            Ok(())
        }
        _ => Err("session evidence structural status is invalid".into()),
    }
}

fn validate_signal(signal: &Value, sequences: &BTreeSet<u64>) -> Result<(), String> {
    exact_keys(
        signal,
        &[
            "kind",
            "status",
            "severity",
            "eventSeq",
            "path",
            "authority",
        ],
        "signal",
    )?;
    if !matches!(
        signal["kind"].as_str(),
        Some(
            "structural_attention_edit"
                | "error_on_structural_attention"
                | "unverified_structural_attention_edit"
        )
    ) || signal["status"] != "observed"
        || signal["severity"] != "review"
        || signal["authority"] != "advisory_only"
        || !signal["eventSeq"]
            .as_u64()
            .is_some_and(|seq| sequences.contains(&seq))
        || signal["path"]
            .as_str()
            .is_none_or(|path| normalize_artifact_path(path).as_deref() != Some(path))
    {
        return Err("session evidence advisory signal is invalid".into());
    }
    Ok(())
}

fn exact_keys(value: &Value, expected: &[&str], label: &str) -> Result<(), String> {
    let actual = value
        .as_object()
        .ok_or_else(|| format!("session evidence {label} must be an object"))?
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(format!("session evidence {label} fields are not closed"));
    }
    Ok(())
}

fn is_grade(value: Option<&str>) -> bool {
    matches!(value, Some("exact" | "estimated" | "unavailable"))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(is_lower_hex)
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

fn coarse_harness_family(value: &str) -> &'static str {
    let normalized = value.to_ascii_lowercase();
    if normalized.contains("codex") {
        "codex"
    } else if normalized.contains("claude") {
        "claude"
    } else {
        "other"
    }
}

fn validate_trace(trace: &Value) -> Result<(), AdapterError> {
    if trace["version"] != 1 {
        return Err(AdapterError::Contract(
            "unsupported session trace schema; expected Mindwalk trace version 1".into(),
        ));
    }
    let session = trace["session"].as_object().ok_or_else(|| {
        AdapterError::Contract("session trace is missing session metadata".into())
    })?;
    if session
        .get("id")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
        || session
            .get("harness")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    {
        return Err(AdapterError::Contract(
            "session trace identity or harness is invalid".into(),
        ));
    }
    let events = trace["events"]
        .as_array()
        .ok_or_else(|| AdapterError::Contract("session trace is missing events".into()))?;
    if events.is_empty() || events.len() > 1_000_000 {
        return Err(AdapterError::Contract(
            "session trace event count is outside the supported range".into(),
        ));
    }
    for event in events {
        if event["seq"].as_u64().is_none()
            || event["tool"].as_str().is_none()
            || event["action"].as_str().is_none()
            || event["isError"].as_bool().is_none()
            || event["targets"].as_array().is_none()
            || event["targets"]
                .as_array()
                .is_some_and(|targets| targets.len() > 10_000)
        {
            return Err(AdapterError::Contract(
                "session trace contains an invalid event".into(),
            ));
        }
        for target in event["targets"].as_array().unwrap() {
            if target["path"].as_str().is_none_or(str::is_empty)
                || target["touch"].as_str().is_none()
            {
                return Err(AdapterError::Contract(
                    "session trace contains an invalid target".into(),
                ));
            }
        }
    }
    Ok(())
}

fn build_hotspot_index(value: &Value) -> Result<BTreeMap<String, Value>, AdapterError> {
    let files = value
        .get("files")
        .or_else(|| value.get("file_details"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AdapterError::Contract("Sentrux enrichment is missing files or file_details".into())
        })?;
    let mut result = BTreeMap::new();
    for file in files {
        let path = file["path"].as_str().ok_or_else(|| {
            AdapterError::Contract("Sentrux enrichment contains an invalid file path".into())
        })?;
        let normalized = normalize_artifact_path(path).ok_or_else(|| {
            AdapterError::Contract("Sentrux enrichment path is not repository-relative".into())
        })?;
        if result.insert(normalized, file.clone()).is_some() {
            return Err(AdapterError::Contract(
                "Sentrux enrichment contains duplicate normalized paths".into(),
            ));
        }
    }
    Ok(result)
}

fn normalize_target(repo: &Path, session_cwd: &Path, raw: &str) -> Option<String> {
    if raw.contains('\0') {
        return None;
    }
    let raw_path = PathBuf::from(raw);
    let candidate = if raw_path.is_absolute() {
        raw_path
    } else {
        session_cwd.join(raw_path)
    };
    let cleaned = clean_path(&candidate)?;
    let boundary_checked = nearest_existing_ancestor(&cleaned)
        .and_then(|ancestor| fs::canonicalize(ancestor).ok())
        .is_some_and(|ancestor| strip_prefix_portable(&ancestor, repo).is_some());
    if !boundary_checked {
        return None;
    }
    let relative = strip_prefix_portable(&cleaned, repo)?;
    normalize_artifact_path(&relative.to_string_lossy())
}

fn clean_path(path: &Path) -> Option<PathBuf> {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                result.push(component.as_os_str())
            }
            Component::CurDir => {}
            Component::ParentDir if !result.pop() => return None,
            Component::ParentDir => {}
        }
    }
    Some(result)
}

fn nearest_existing_ancestor(path: &Path) -> Option<&Path> {
    path.ancestors().find(|ancestor| ancestor.exists())
}

fn strip_prefix_portable(path: &Path, root: &Path) -> Option<PathBuf> {
    let path_components = path.components().collect::<Vec<_>>();
    let root_components = root.components().collect::<Vec<_>>();
    if root_components.len() > path_components.len() {
        return None;
    }
    for (actual, expected) in path_components.iter().zip(&root_components) {
        let actual = actual.as_os_str().to_string_lossy();
        let expected = expected.as_os_str().to_string_lossy();
        if cfg!(windows) {
            if !actual.eq_ignore_ascii_case(&expected) {
                return None;
            }
        } else if actual != expected {
            return None;
        }
    }
    let mut relative = PathBuf::new();
    for component in &path_components[root_components.len()..] {
        relative.push(component.as_os_str());
    }
    Some(relative)
}

fn normalize_artifact_path(path: &str) -> Option<String> {
    let replaced = path.replace('\\', "/");
    let trimmed = replaced.trim().trim_start_matches("./");
    if trimmed.is_empty() || Path::new(trimmed).is_absolute() {
        return None;
    }
    let mut components = Vec::new();
    for component in trimmed.split('/') {
        match component {
            "" | "." => {}
            ".." => return None,
            value => components.push(value),
        }
    }
    (!components.is_empty()).then(|| {
        let normalized = components.join("/");
        if cfg!(windows) {
            normalized.to_lowercase()
        } else {
            normalized
        }
    })
}

fn normalized_action(value: Option<&str>) -> &'static str {
    match value {
        Some("search") => "search",
        Some("read") => "read",
        Some("edit") => "edit",
        Some("exec") => "exec",
        Some("verify") => "verify",
        _ => "other",
    }
}

fn normalized_touch(value: Option<&str>) -> &'static str {
    match value {
        Some("read") => "read",
        Some("edit") => "edit",
        _ => "hit",
    }
}

fn tool_family(value: Option<&str>) -> &'static str {
    let value = value.unwrap_or("").to_ascii_lowercase();
    if value.contains("apply_patch") || value.contains("write") || value.contains("edit") {
        "edit"
    } else if value.contains("exec") || value.contains("command") || value.contains("shell") {
        "shell"
    } else if value.contains("read") || value.contains("view") || value.contains("open") {
        "read"
    } else if value.contains("search") || value.contains("find") || value == "rg" {
        "search"
    } else if value.contains("agent") || value.contains("wait") || value.contains("message") {
        "orchestration"
    } else {
        "other"
    }
}

fn grade(value: Option<&str>) -> &'static str {
    match value {
        Some("exact") => "exact",
        Some("estimated") => "estimated",
        _ => "unavailable",
    }
}

fn u64_any(value: &Value, fields: &[&str]) -> u64 {
    fields
        .iter()
        .find_map(|field| value.get(field).and_then(Value::as_u64))
        .unwrap_or(0)
}

fn f64_any(value: &Value, fields: &[&str]) -> f64 {
    fields
        .iter()
        .find_map(|field| value.get(field).and_then(Value::as_f64))
        .unwrap_or(0.0)
}

fn structural_attention(hotspot: &Value) -> bool {
    u64_any(hotspot, &["maxComplexity", "max_complexity"]) >= 20
        || hotspot["git"]["churn"].as_u64().unwrap_or(0) >= 5
        || hotspot["git"]["dirty"].as_bool().unwrap_or(false)
}

fn signal(kind: &str, event_seq: u64, path: &str, severity: &str) -> Value {
    json!({
        "kind":kind,
        "status":"observed",
        "severity":severity,
        "eventSeq":event_seq,
        "path":path,
        "authority":"advisory_only"
    })
}

fn read_bounded_regular(path: &Path, max_bytes: u64, label: &str) -> Result<Vec<u8>, AdapterError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| AdapterError::Io(format!("cannot inspect {label}: {error}")))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(AdapterError::Contract(format!(
            "{label} must be a regular file"
        )));
    }
    if metadata.len() > max_bytes {
        return Err(AdapterError::Contract(format!(
            "{label} exceeds the {max_bytes}-byte limit"
        )));
    }
    fs::read(path).map_err(|error| AdapterError::Io(format!("cannot read {label}: {error}")))
}

fn write_new_artifact(path: &Path, bytes: &[u8]) -> Result<(), AdapterError> {
    if path.exists() {
        return Err(AdapterError::Usage(format!(
            "output already exists: {}",
            path.display()
        )));
    }
    let parent = path
        .parent()
        .filter(|parent| parent.is_dir())
        .ok_or_else(|| AdapterError::Usage("output parent directory does not exist".into()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AdapterError::Usage("output file name is invalid".into()))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| AdapterError::Io(error.to_string()))?
        .as_nanos();
    let temporary = parent.join(format!(".{name}.tmp-{}-{nonce}", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| AdapterError::Io(format!("cannot create staged output: {error}")))?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| AdapterError::Io(format!("cannot write staged output: {error}")))?;
        fs::rename(&temporary, path)
            .map_err(|error| AdapterError::Io(format!("cannot publish output: {error}")))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
