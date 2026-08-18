use std::process::Output;

use serde_json::{json, Value};

use super::sentrux_gate::Violation;
use crate::capability::sha256_hex;

pub(crate) const MAX_COMMAND_EVIDENCE_BYTES: usize = 1024 * 1024;
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

impl SentruxCommand {
    pub(crate) fn from_native(run: super::sentrux_gate::EngineRun, subcommand: &str) -> Self {
        let stdout_bytes = run.stdout.into_bytes();
        let output_summary = OutputSummary::from_bytes(&stdout_bytes, &[]);
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
        }
    }

    pub(crate) fn from_external(output: Output, subcommand: &str) -> Self {
        let output_summary = OutputSummary::from_bytes(&output.stdout, &output.stderr);
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
        }
    }

    pub(crate) fn from_json(stdout: Vec<u8>, subcommand: &str) -> Self {
        let output_summary = OutputSummary::from_bytes(&stdout, &[]);
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
        "outputSummary":command.output_summary.to_json(&command.stdout, &command.stderr)
    })
}
