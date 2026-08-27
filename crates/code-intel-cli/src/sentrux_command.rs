use std::process::Output;

use serde_json::{json, Value};

use super::sentrux_gate::Violation;
use crate::capability::sha256_hex;

// This was 1 MiB at authoring (commit b5bb8f04, PR #286/#285) and never
// revisited since. Its purpose (per #286's own description) is to stop a
// truncated capture being *silently* treated as a complete one -- not to
// cap how large a genuinely successful capability's real output may be.
// `sentrux.dsm`'s real, honest output on this repository is already ~3.8MB
// once `dsm_edges` actually reports coupling for a repo this size (issue
// #376/DR-0010 -- previously it was near-empty only because `dsm_edges`
// was structurally blind to this repo's own coupling, which is the bug
// #376 fixes); `sentrux.scan`'s is ~325KB. 1 MiB was stale relative to both
// well before #376, and 16 MiB keeps ~4x headroom over dsm's current real
// size (against the DAG's own 100MB total run budget) while still catching
// a genuinely pathological/runaway command.
pub(crate) const MAX_COMMAND_EVIDENCE_BYTES: usize = 16 * 1024 * 1024;
const MAX_COMMAND_PREVIEW_BYTES: usize = 8 * 1024;

pub(crate) struct SentruxCommand {
    pub(crate) argv: Vec<String>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) success: bool,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) violations: Vec<Violation>,
    pub(crate) governed: bool,
    pub(crate) output_summary: OutputSummary,
    // Issue #383: `stdout` above is always `bounded_text`-truncated to
    // `MAX_COMMAND_PREVIEW_BYTES` (8KB) -- a small snippet for human
    // diagnostics. Machine consumers of a capability's real structured
    // output (`capability_structured_data` and everything downstream of
    // it) must not re-parse that truncated preview: any real JSON output
    // over 8KB silently failed to parse and became `Value::Null`, even
    // though `status` correctly reported `"succeeded"`/`"complete"`. This
    // field is the full, unbounded output parsed once at construction time
    // (`structured_data_from_full`), kept *alongside* the bounded preview,
    // not instead of it -- `None` when the output is not `complete()` (to
    // avoid parsing a genuinely truncated document as if it were whole) or
    // is not a JSON object/array.
    pub(crate) structured_stdout: Option<Value>,
}

#[derive(Clone, Debug)]
pub(crate) struct OutputSummary {
    stdout_bytes: usize,
    stdout_sha256: String,
    stderr_bytes: usize,
    stderr_sha256: String,
}

impl OutputSummary {
    pub(crate) fn from_bytes(stdout: &[u8], stderr: &[u8]) -> Self {
        Self {
            stdout_bytes: stdout.len(),
            stdout_sha256: sha256_hex(stdout),
            stderr_bytes: stderr.len(),
            stderr_sha256: sha256_hex(stderr),
        }
    }

    pub(crate) fn complete(&self) -> bool {
        self.stdout_bytes <= MAX_COMMAND_EVIDENCE_BYTES
            && self.stderr_bytes <= MAX_COMMAND_EVIDENCE_BYTES
    }

    pub(crate) fn to_json(&self, stdout_preview: &str, stderr_preview: &str) -> Value {
        json!({
            "authority":"metadata_only",
            "complete":self.complete(),
            "bounded":!self.complete(),
            "limitBytes":MAX_COMMAND_EVIDENCE_BYTES,
            "totalBytes":self.stdout_bytes + self.stderr_bytes,
            "stdout":{
                "bytes":self.stdout_bytes,
                "sha256":self.stdout_sha256,
                "preview":stdout_preview,
                "previewBytes":stdout_preview.len()
            },
            "stderr":{
                "bytes":self.stderr_bytes,
                "sha256":self.stderr_sha256,
                "preview":stderr_preview,
                "previewBytes":stderr_preview.len()
            },
            "note":"preview is non-authoritative; consumers must use the artifact metadata"
        })
    }

    pub(crate) fn from_metadata(summary: &serde_json::Map<String, Value>) -> Self {
        fn digest(summary: &serde_json::Map<String, Value>, stream: &str) -> String {
            summary[stream]["sha256"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        }
        fn bytes(summary: &serde_json::Map<String, Value>, stream: &str) -> usize {
            summary[stream]["bytes"].as_u64().unwrap_or(0) as usize
        }
        Self {
            stdout_bytes: bytes(summary, "stdout"),
            stdout_sha256: digest(summary, "stdout"),
            stderr_bytes: bytes(summary, "stderr"),
            stderr_sha256: digest(summary, "stderr"),
        }
    }
}

fn bounded_text(bytes: &[u8]) -> String {
    let end = bytes.len().min(MAX_COMMAND_PREVIEW_BYTES);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Issue #383: parse the *full, unbounded* command output as JSON once, at
/// construction time, independently of `bounded_text`'s 8KB preview
/// truncation. `complete` must be `output_summary.complete()` -- when the
/// capture itself was bounded by `MAX_COMMAND_EVIDENCE_BYTES` the bytes are
/// not the real document at all, so parsing them would either fail (honest
/// `None`) or, worse, occasionally succeed on a coincidentally-valid partial
/// prefix (dishonest). Only object/array results are kept, matching every
/// other structured-data consumer in this file: a scalar or `null` top-level
/// value is not a capability payload.
fn structured_data_from_full(bytes: &[u8], complete: bool) -> Option<Value> {
    if !complete {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let value = serde_json::from_str::<Value>(text).ok()?;
    (value.is_object() || value.is_array()).then_some(value)
}

impl SentruxCommand {
    pub(crate) fn violations_json(&self) -> Value {
        json!(self
            .violations
            .iter()
            .map(Violation::to_json)
            .collect::<Vec<_>>())
    }

    pub(crate) fn violations_from_json(value: Option<&Value>) -> Vec<Violation> {
        value
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        Some(Violation {
                            rule: item["rule"].as_str()?.to_owned(),
                            message: item["message"].as_str()?.to_owned(),
                            targets: item["targets"]
                                .as_array()?
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn from_native(run: super::sentrux_gate::EngineRun, subcommand: &str) -> Self {
        let stdout_bytes = run.stdout.into_bytes();
        let output_summary = OutputSummary::from_bytes(&stdout_bytes, &[]);
        let structured_stdout =
            structured_data_from_full(&stdout_bytes, output_summary.complete());
        Self {
            argv: vec![
                "code-intel".into(),
                "sentrux".into(),
                subcommand.into(),
                ".".into(),
            ],
            exit_code: Some(if run.success { 0 } else { 1 }),
            success: run.success,
            stdout: bounded_text(&stdout_bytes),
            stderr: String::new(),
            violations: run.violations,
            governed: run.governed,
            output_summary,
            structured_stdout,
        }
    }

    pub(crate) fn from_external(output: Output, subcommand: &str) -> Self {
        let output_summary = OutputSummary::from_bytes(&output.stdout, &output.stderr);
        let structured_stdout =
            structured_data_from_full(&output.stdout, output_summary.complete());
        let stdout_full = String::from_utf8_lossy(&output.stdout).into_owned();
        let violations = if output.status.success() {
            Vec::new()
        } else {
            stdout_full
                .lines()
                .filter_map(|line| line.strip_prefix("- "))
                .map(str::trim)
                .filter(|message| !message.is_empty())
                .take(32)
                .map(|message| Violation {
                    rule: format!("sentrux_{subcommand}"),
                    message: message.chars().take(1024).collect(),
                    targets: Vec::new(),
                })
                .collect()
        };
        Self {
            argv: vec!["sentrux".into(), subcommand.into(), ".".into()],
            exit_code: output.status.code(),
            success: output.status.success(),
            stdout: bounded_text(&output.stdout),
            stderr: bounded_text(&output.stderr),
            violations,
            governed: true,
            output_summary,
            structured_stdout,
        }
    }

    pub(crate) fn from_json(stdout: Vec<u8>, subcommand: &str) -> Self {
        let output_summary = OutputSummary::from_bytes(&stdout, &[]);
        let structured_stdout = structured_data_from_full(&stdout, output_summary.complete());
        Self {
            argv: vec![
                "code-intel".into(),
                "sentrux".into(),
                subcommand.into(),
                ".".into(),
            ],
            exit_code: Some(0),
            success: true,
            stdout: bounded_text(&stdout),
            stderr: String::new(),
            violations: Vec::new(),
            governed: true,
            output_summary,
            structured_stdout,
        }
    }
}

pub(crate) fn command_evidence(subcommand: &str, command: &SentruxCommand) -> Value {
    json!({
        "id":subcommand,
        "argv":command.argv,
        "exitCode":command.exit_code,
        "success":command.success,
        "stdout":command.stdout,
        "stderr":command.stderr,
        // Issue #383: the full, unbounded structured payload -- never
        // re-derived by reparsing `stdout` above (that's the bounded 8KB
        // preview). `Value::Null` when the command's real output was not a
        // complete JSON object/array (bounded capture, plain-text output,
        // or a non-JSON command like `check`/`gate`).
        "structuredData":command.structured_stdout.clone().unwrap_or(Value::Null)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A JSON object that serializes comfortably over `MAX_COMMAND_PREVIEW_BYTES`
    /// (8KB), the cap issue #383 found `structuredData` was silently truncated
    /// against. `min_bytes` bounds the filler alone, so the fixture is reliably
    /// larger than the cap regardless of the surrounding object's own bytes.
    fn over_preview_cap_json(min_bytes: usize) -> Value {
        json!({
            "marker": "issue-383-over-8kb-fixture",
            "filler": "x".repeat(min_bytes),
            "nested": {"a": 1, "b": [1, 2, 3], "c": null},
        })
    }

    #[test]
    fn structured_stdout_round_trips_a_payload_over_the_8kb_preview_cap() {
        let value = over_preview_cap_json(MAX_COMMAND_PREVIEW_BYTES + 4096);
        let bytes = serde_json::to_vec(&value).expect("serialize fixture");
        assert!(
            bytes.len() > MAX_COMMAND_PREVIEW_BYTES,
            "fixture must exceed the old 8KB preview cap to reproduce #383"
        );

        let command = SentruxCommand::from_json(bytes, "scan");

        // The bounded human preview stays capped at 8KB...
        assert_eq!(command.stdout.len(), MAX_COMMAND_PREVIEW_BYTES);
        // ...but the structured channel carries the whole document, not a
        // reparse of the truncated preview (which would fail to parse and
        // silently become `None`/`Value::Null`, #383's exact bug).
        assert_eq!(
            command.structured_stdout.as_ref(),
            Some(&value),
            "structuredData must reflect the full parsed output beyond the preview cap"
        );

        // `command_evidence` must embed the full value directly, and keep the
        // bounded preview as a separate, still-8KB-capped field.
        let evidence = command_evidence("scan", &command);
        assert_eq!(evidence["structuredData"], value);
        assert_eq!(
            evidence["stdout"].as_str().unwrap().len(),
            MAX_COMMAND_PREVIEW_BYTES
        );
    }

    #[test]
    fn structured_data_from_full_is_none_when_capture_was_bounded() {
        // `complete=false` simulates `output_summary.complete()` being false
        // (the real bytes exceeded `MAX_COMMAND_EVIDENCE_BYTES`): parsing must
        // not be attempted on a capture that is not the real document.
        let value = over_preview_cap_json(1024);
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(structured_data_from_full(&bytes, false).is_none());
    }

    #[test]
    fn structured_data_from_full_ignores_non_object_non_array_top_level_values() {
        assert!(structured_data_from_full(b"42", true).is_none());
        assert!(structured_data_from_full(b"\"just a string\"", true).is_none());
        assert!(structured_data_from_full(b"null", true).is_none());
    }

    #[test]
    fn structured_data_from_full_is_none_for_plain_text_output() {
        // `check`/`gate` emit human text, not JSON -- `structured_stdout` must
        // stay honestly `None`, never coerce text into a fabricated payload.
        let bytes = b"Quality: 9000 -> 9200\nCoupling: 4 -> 3\n".to_vec();
        assert!(structured_data_from_full(&bytes, true).is_none());
    }

    #[test]
    fn from_native_also_populates_structured_stdout_over_8kb() {
        let value = over_preview_cap_json(MAX_COMMAND_PREVIEW_BYTES + 2048);
        let text = serde_json::to_string(&value).expect("serialize fixture");
        assert!(text.len() > MAX_COMMAND_PREVIEW_BYTES);

        let native = SentruxCommand::from_native(
            super::super::sentrux_gate::EngineRun {
                success: true,
                stdout: text,
                violations: Vec::new(),
                governed: true,
            },
            "scan",
        );
        assert_eq!(native.structured_stdout.as_ref(), Some(&value));
    }
}
