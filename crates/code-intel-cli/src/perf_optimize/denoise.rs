//! #301's denoising wrapper: runs a real eval-command N times (default 5)
//! and reports the median, so a single noisy sample never gets misread as
//! "the optimization worked." weco itself only parses a single `metric:
//! value` line per step, so the median has to be computed on our side and
//! re-printed in the same shape weco expects — see `run_raw` in
//! `super::denoise_eval_cli` for the subcommand that wraps this.

use std::process::Command;

/// #301's default repeat count for the denoising wrapper.
pub(crate) const DEFAULT_DENOISE_N: usize = 5;

#[derive(Debug, PartialEq)]
pub(crate) enum DenoiseError {
    Io {
        attempt: usize,
        message: String,
    },
    NonZeroExit {
        attempt: usize,
        exit_code: Option<i32>,
        stderr: String,
    },
    Unparsable {
        attempt: usize,
        stdout: String,
    },
    InconsistentMetricName {
        first: String,
        attempt: usize,
        found: String,
    },
    ZeroSamplesRequested,
}

impl std::fmt::Display for DenoiseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { attempt, message } => {
                write!(f, "eval-command attempt {attempt} failed to spawn: {message}")
            }
            Self::NonZeroExit {
                attempt,
                exit_code,
                stderr,
            } => write!(
                f,
                "eval-command attempt {attempt} exited non-zero ({exit_code:?}): {stderr}"
            ),
            Self::Unparsable { attempt, stdout } => write!(
                f,
                "eval-command attempt {attempt} did not print a 'metric_name: value' line: {stdout:?}"
            ),
            Self::InconsistentMetricName {
                first,
                attempt,
                found,
            } => write!(
                f,
                "eval-command's metric name changed between attempts: attempt 1 printed {first:?}, attempt {attempt} printed {found:?}"
            ),
            Self::ZeroSamplesRequested => write!(f, "denoising requires at least one sample"),
        }
    }
}

#[derive(Debug)]
pub(crate) struct DenoiseResult {
    pub(crate) metric_name: String,
    pub(crate) median: f64,
    pub(crate) samples: Vec<f64>,
}

/// Parses weco's expected eval-command output format: the last `name: number`
/// pair on the line, trimmed. Splitting on the *last* colon (rather than the
/// first) tolerates a metric name that itself contains a colon.
pub(crate) fn parse_metric_line(line: &str) -> Option<(String, f64)> {
    let line = line.trim();
    let colon = line.rfind(':')?;
    let name = line[..colon].trim();
    let value = line[colon + 1..].trim();
    if name.is_empty() {
        return None;
    }
    value
        .parse::<f64>()
        .ok()
        .map(|value| (name.to_string(), value))
}

/// The last parseable `name: value` line across every line of `stdout` — a
/// command may print progress noise before its final metric line.
fn parse_last_metric_line(stdout: &str) -> Option<(String, f64)> {
    stdout.lines().rev().find_map(parse_metric_line)
}

pub(crate) fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("eval-command output is never NaN"));
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

/// Runs `shell_command` `n` times, parses each attempt's final metric line,
/// and returns the median. Every attempt must agree on the metric name.
pub(crate) fn run_denoised(shell_command: &str, n: usize) -> Result<DenoiseResult, DenoiseError> {
    if n == 0 {
        return Err(DenoiseError::ZeroSamplesRequested);
    }
    let mut metric_name: Option<String> = None;
    let mut samples = Vec::with_capacity(n);
    for attempt in 1..=n {
        let output = spawn_shell(shell_command)
            .output()
            .map_err(|error| DenoiseError::Io {
                attempt,
                message: error.to_string(),
            })?;
        if !output.status.success() {
            return Err(DenoiseError::NonZeroExit {
                attempt,
                exit_code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let (name, value) = parse_last_metric_line(&stdout).ok_or(DenoiseError::Unparsable {
            attempt,
            stdout: stdout.clone(),
        })?;
        match &metric_name {
            None => metric_name = Some(name),
            Some(first) if *first == name => {}
            Some(first) => {
                return Err(DenoiseError::InconsistentMetricName {
                    first: first.clone(),
                    attempt,
                    found: name,
                })
            }
        }
        samples.push(value);
    }
    Ok(DenoiseResult {
        metric_name: metric_name.expect("at least one sample was collected"),
        median: median(&samples),
        samples,
    })
}

fn spawn_shell(shell_command: &str) -> Command {
    if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/C", shell_command]);
        command
    } else {
        let mut command = Command::new("sh");
        command.args(["-c", shell_command]);
        command
    }
}

#[cfg(test)]
#[path = "denoise_tests.rs"]
mod tests;
