//! #301: invokes `weco` (WecoAI's AIDE-style perf-optimize CLI) for one
//! optimization pass against a benchmark target, producing a denoised,
//! budget-bounded result report. No application changes, no PR — that's
//! #302. See the issue's grilling notes for why the budget/timeout/report
//! shape here differs from a naive read of #157: weco has no per-step cost
//! signal, so budget maps to `--steps` via a fixed empirical seconds-per-step
//! assumption, and the wall-clock backstop asks weco to stop itself
//! (`weco run stop <run-id>`) rather than reusing the DAG's generic
//! hard-kill node timeout.
//!
//! **weco is not a local-only tool** (confirmed by reading weco-cli's own
//! source, since the original design assumed otherwise): its optimization
//! decision loop runs server-side, polled over HTTP with a heartbeat thread
//! keeping the run alive on weco's backend. A BYOK key (`OPENAI_API_KEY` /
//! `ANTHROPIC_API_KEY` / `GEMINI_API_KEY`) only pays for LLM generation --
//! weco's own account token (`WECO_API_KEY`, distinct from BYOK) is required
//! unconditionally to create or track a run at all. `weco_availability`
//! checks both, independently, via `doctor_bootstrap`'s
//! `checks.weco.{byokConfigured,accountConfigured}`.
//!
//! `run status`/`run diff` (not stdout-scraping) are the verified source for
//! step counts, the best metric, and the winning candidate's diff -- see
//! `weco_run_status` and `best_candidate_diff` below for the exact shapes,
//! confirmed against weco-cli's source rather than guessed.
//!
//! Exit codes: 0 success (a candidate was measured, threshold met or not —
//! "didn't clear the bar" is still a full report, not a failure); 64 usage
//! error; 69 unavailable (no `--eval-command`/`--source`, the eval command
//! itself can't be run, or weco is missing/not fully authenticated — the
//! same collapse `doctor_provider_rows.rs` already uses for "not ready");
//! 74 weco ran but produced no usable result (no run id ever printed, the
//! status query itself failed, or zero steps completed before it stopped);
//! 76 the wall-clock budget can't afford even one step, so no report was
//! attempted (#157's own precedent: a budget too small for the first unit
//! of work is a failure, not a degraded success).

mod denoise;
mod denoise_eval_cli;
mod report;
mod steps;
mod weco_process;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

#[path = "../tool_path.rs"]
mod tool_path;

use report::{Goal, StoppedBy};

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    run_dispatch(raw, "run")
}

pub(crate) fn run_denoise_eval_raw(raw: &[String]) -> i32 {
    denoise_eval_cli::run_raw(raw)
}

fn run_dispatch(raw: &[String], _subcommand: &str) -> i32 {
    let cli = match Cli::parse(raw) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}");
            return 64;
        }
    };
    execute(&cli)
}

#[derive(Debug)]
struct Cli {
    repo: PathBuf,
    target: String,
    source: Option<PathBuf>,
    eval_command: Option<String>,
    metric: String,
    goal: Goal,
    min_improvement_percent: f64,
    model: Option<String>,
    budget_wall_clock_seconds: Option<u64>,
    seconds_per_step: u64,
    doctor_tool_path_prefix: Option<PathBuf>,
    denoise_n: usize,
    grace_period_seconds: u64,
}

/// Grace period given to weco to exit on its own after `weco run stop`
/// before this falls back to a hard kill. Generous relative to how quickly
/// a well-behaved process should react to a stop request, tiny relative to
/// the wall-clock budget it's bounded by.
const DEFAULT_GRACE_PERIOD_SECONDS: u64 = 30;

impl Cli {
    fn parse(raw: &[String]) -> Result<Self, String> {
        let mut repo = None;
        let mut target = None;
        let mut source = None;
        let mut eval_command = None;
        let mut metric = None;
        let mut goal = None;
        let mut min_improvement_percent = report::DEFAULT_MIN_IMPROVEMENT_PERCENT;
        let mut model = None;
        let mut budget_wall_clock_seconds = None;
        let mut seconds_per_step = steps::DEFAULT_SECONDS_PER_STEP;
        let mut doctor_tool_path_prefix = None;
        let mut denoise_n = denoise::DEFAULT_DENOISE_N;
        let mut grace_period_seconds = DEFAULT_GRACE_PERIOD_SECONDS;

        let mut iter = raw.iter();
        while let Some(argument) = iter.next() {
            match argument.as_str() {
                "--repo" => repo = Some(PathBuf::from(next_value(&mut iter, "--repo")?)),
                "--target" => target = Some(next_value(&mut iter, "--target")?.clone()),
                "--source" => source = Some(PathBuf::from(next_value(&mut iter, "--source")?)),
                "--eval-command" => {
                    eval_command = Some(next_value(&mut iter, "--eval-command")?.clone())
                }
                "--metric" => metric = Some(next_value(&mut iter, "--metric")?.clone()),
                "--goal" => goal = Some(Goal::parse(next_value(&mut iter, "--goal")?)?),
                "--min-improvement" => {
                    min_improvement_percent =
                        parse_percent(next_value(&mut iter, "--min-improvement")?)?
                }
                "--model" => model = Some(next_value(&mut iter, "--model")?.clone()),
                "--budget-wall-clock" => {
                    budget_wall_clock_seconds = Some(
                        next_value(&mut iter, "--budget-wall-clock")?
                            .parse::<u64>()
                            .map_err(|_| "--budget-wall-clock must be an integer".to_string())?,
                    )
                }
                "--seconds-per-step" => {
                    seconds_per_step = next_value(&mut iter, "--seconds-per-step")?
                        .parse::<u64>()
                        .map_err(|_| "--seconds-per-step must be an integer".to_string())?
                }
                "--doctor-tool-path-prefix" => {
                    doctor_tool_path_prefix = Some(PathBuf::from(next_value(
                        &mut iter,
                        "--doctor-tool-path-prefix",
                    )?))
                }
                "--denoise-n" => {
                    denoise_n = next_value(&mut iter, "--denoise-n")?
                        .parse::<usize>()
                        .map_err(|_| "--denoise-n must be a positive integer".to_string())?
                }
                "--grace-period-seconds" => {
                    grace_period_seconds = next_value(&mut iter, "--grace-period-seconds")?
                        .parse::<u64>()
                        .map_err(|_| "--grace-period-seconds must be an integer".to_string())?
                }
                other => return Err(format!("unknown perf-optimize run argument: {other}")),
            }
        }

        Ok(Self {
            repo: repo.ok_or("--repo is required")?,
            target: target.ok_or("--target is required")?,
            source,
            eval_command,
            metric: metric.ok_or("--metric is required")?,
            goal: goal.ok_or("--goal is required")?,
            min_improvement_percent,
            model,
            budget_wall_clock_seconds,
            seconds_per_step,
            doctor_tool_path_prefix,
            denoise_n,
            grace_period_seconds,
        })
    }
}

fn next_value<'a>(
    iter: &mut std::slice::Iter<'a, String>,
    flag: &'static str,
) -> Result<&'a String, String> {
    iter.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn parse_percent(value: &str) -> Result<f64, String> {
    value
        .trim_end_matches('%')
        .parse::<f64>()
        .map_err(|_| format!("value must be a percentage like '5' or '5%': {value:?}"))
}

fn status_report(status: &str, reason: &str, exit_code: i32) -> i32 {
    let report = serde_json::json!({
        "schema": "code-intel-perf-optimize-run.v1",
        "status": status,
        "reason": reason,
    });
    println!(
        "{}",
        serde_json::to_string(&report).expect("report serializes")
    );
    exit_code
}

fn unavailable(reason: &str) -> i32 {
    status_report("unavailable", reason, 69)
}

fn failed(reason: &str) -> i32 {
    status_report("failed", reason, 76)
}

fn pipeline_root() -> PathBuf {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| {
            cwd.ancestors()
                .find(|dir| {
                    dir.join("orchestration")
                        .join("integrations.json")
                        .is_file()
                })
                .map(Path::to_path_buf)
        })
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Presence comes from `tool_path::locate` directly, returned as the
/// resolved path so `execute` never has to look it up a second time -- one
/// answer to "is weco on PATH", not two that could disagree or duplicate the
/// search. BYOK and account-token status still come from
/// `doctor_bootstrap::observe`'s `checks.weco.{byokConfigured,
/// accountConfigured}` -- those checks belong to #300, not duplicated here.
/// weco's own run loop is server-tracked (verified against its source, #301
/// research): the account token is required unconditionally, in addition to
/// -- not instead of -- a BYOK key that only pays for LLM generation.
fn weco_availability(cli: &Cli) -> Result<PathBuf, String> {
    let prefix = cli.doctor_tool_path_prefix.as_deref();
    let Some(weco_binary) = tool_path::locate("weco", prefix) else {
        return Err("weco is not on PATH".to_string());
    };
    let mut options = crate::doctor_bootstrap::Options::new(pipeline_root());
    options.tool_path_prefix = cli.doctor_tool_path_prefix.clone();
    let observation = crate::doctor_bootstrap::observe(&options)?;
    let byok_configured = observation
        .pointer("/checks/weco/byokConfigured")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let account_configured = observation
        .pointer("/checks/weco/accountConfigured")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !byok_configured {
        return Err(
            "weco is present but no BYOK provider key is configured (OPENAI_API_KEY/ANTHROPIC_API_KEY/GEMINI_API_KEY)"
                .to_string(),
        );
    }
    if !account_configured {
        return Err(
            "weco is present but no weco.ai account is configured (WECO_API_KEY) -- required by weco's server-tracked run loop even with a BYOK key"
                .to_string(),
        );
    }
    Ok(weco_binary)
}

fn execute(cli: &Cli) -> i32 {
    // Checked cheapest and most deterministic first: budget viability is a
    // pure function of CLI args, so a misconfigured budget is reported
    // before anything that needs weco (or even the eval-command) to be
    // reachable at all.
    let default_wall_clock = crate::budget::Budget::with_defaults().wall_clock_limit();
    let wall_clock_budget_seconds = cli.budget_wall_clock_seconds.unwrap_or(default_wall_clock);
    let steps_planned =
        steps::steps_from_wall_clock_budget(wall_clock_budget_seconds, cli.seconds_per_step);
    if steps_planned == 0 {
        return failed(&format!(
            "wall-clock budget ({wall_clock_budget_seconds}s) cannot afford even one step at {seconds_per_step}s/step",
            seconds_per_step = cli.seconds_per_step
        ));
    }

    let Some(eval_command) = &cli.eval_command else {
        return unavailable(
            "no --eval-command provided: target has no known-reusable benchmark/eval script",
        );
    };
    // #301 correction: weco requires a source file to mutate (its own
    // `-s/--source`, a hard requirement -- confirmed against a real install,
    // not a guess) -- a target with no known file to optimize is no more
    // "usable" than one with no eval-command.
    let Some(source) = &cli.source else {
        return unavailable("no --source provided: target has no known file for weco to optimize");
    };
    let weco_binary = match weco_availability(cli) {
        Ok(weco_binary) => weco_binary,
        Err(reason) => return unavailable(&reason),
    };

    let baseline_result = match denoise::run_denoised(eval_command, cli.denoise_n) {
        Ok(result) => result,
        Err(error) => return unavailable(&format!("baseline eval-command failed: {error}")),
    };
    if baseline_result.metric_name != cli.metric {
        return unavailable(&format!(
            "eval-command printed metric {actual:?}, expected --metric {expected:?}",
            actual = baseline_result.metric_name,
            expected = cli.metric
        ));
    }

    let current_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("code-intel"));
    let wrapped_eval_command = format!(
        "{exe} perf-optimize denoise-eval --command {command} --n {n}",
        exe = shell_quote(&current_exe.display().to_string()),
        command = shell_quote(eval_command),
        n = cli.denoise_n
    );

    let mut weco_command = Command::new(&weco_binary);
    weco_command
        .args(["run", "--source"])
        .arg(source)
        .args(["--metric", &cli.metric, "--goal", goal_flag(cli.goal)])
        .args(["--steps", &steps_planned.to_string()])
        .args(["--eval-command", &wrapped_eval_command])
        // "plain" is weco's own machine-readable mode (confirmed via source:
        // a dedicated `[METRIC] Step N: value (best so far: best)` line
        // separate from the raw eval-command echo) -- not currently parsed
        // here (see the status-query comment below for why), but still the
        // right mode for a non-interactive subprocess: the default "rich"
        // mode is an interactive TUI that assumes a real terminal.
        .args(["--output", "plain"])
        .current_dir(&cli.repo);
    if let Some(model) = &cli.model {
        weco_command.args(["--model", model]);
    }

    let outcome = match weco_process::run_weco_with_wall_clock_backstop(
        weco_command,
        Duration::from_secs(wall_clock_budget_seconds),
        &weco_binary,
        Duration::from_secs(cli.grace_period_seconds),
    ) {
        Ok(outcome) => outcome,
        Err(weco_process::WecoRunError::Io(error)) => {
            eprintln!("failed to run weco: {error}");
            return 74;
        }
    };

    // weco's run loop is server-tracked (#301 research, verified against
    // source): `run status` is the authoritative source for how far a run
    // got, not stdout -- weco's own dedicated `[METRIC] Step N: value (best
    // so far: best)` plain-output lines exist and could be scanned, but the
    // server already aggregates exactly this, so querying it once after the
    // process ends is simpler and doesn't require re-deriving an extremum
    // ourselves.
    let Some(run_id) = &outcome.run_id else {
        eprintln!("weco exited without ever printing a run id");
        return 74;
    };
    let status = match weco_run_status(&weco_binary, run_id) {
        Ok(status) => status,
        Err(reason) => {
            eprintln!("failed to query weco run status: {reason}");
            return 74;
        }
    };
    let steps_run = status.current_step;
    let Some(best) = status.best_metric else {
        eprintln!("weco completed zero steps before stopping");
        return 74;
    };

    let stopped_by = if outcome.timed_out {
        Some(StoppedBy::BudgetExhausted)
    } else {
        // Either weco ran every allocated step, or it chose to stop early
        // (converged, gave up) without our backstop intervening -- from our
        // side both look like "the search concluded on its own", the same
        // signal steps_exhausted is meant to carry.
        Some(StoppedBy::StepsExhausted)
    };

    let diff = best_candidate_diff(&weco_binary, run_id);
    let report = report::build_report(
        &cli.target,
        &cli.metric,
        cli.goal,
        baseline_result.median,
        Some(best),
        diff.as_deref(),
        cli.min_improvement_percent,
        steps_run,
        steps_planned,
        stopped_by,
    );
    println!(
        "{}",
        serde_json::to_string(&report).expect("report serializes")
    );
    0
}

fn goal_flag(goal: Goal) -> &'static str {
    match goal {
        Goal::Maximize => "maximize",
        Goal::Minimize => "minimize",
    }
}

/// `weco run status <run-id>`'s fields relevant to this report. Verified
/// against weco-cli's own source (#301 research), not guessed: the command
/// always prints JSON (no `--format` flag needed, unlike `run results`).
struct WecoStatus {
    current_step: u64,
    best_metric: Option<f64>,
}

fn weco_run_status(weco_binary: &Path, run_id: &str) -> Result<WecoStatus, String> {
    let output = Command::new(weco_binary)
        .args(["run", "status", run_id])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "weco run status exited non-zero: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
    Ok(WecoStatus {
        current_step: value
            .get("current_step")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        best_metric: value.get("best_metric").and_then(serde_json::Value::as_f64),
    })
}

/// `weco run diff <run-id> --step best --against baseline` prints a raw
/// unified diff to stdout on success, or `{"error": "..."}` JSON on failure
/// (verified against weco-cli's own source, #301 research -- not the
/// guessed `run status` JSON-field approach this replaced). Best-effort: a
/// missing command, a failed diff, or empty output all just mean no diff in
/// the report, never a failed run.
fn best_candidate_diff(weco_binary: &Path, run_id: &str) -> Option<String> {
    let output = Command::new(weco_binary)
        .args([
            "run",
            "diff",
            run_id,
            "--step",
            "best",
            "--against",
            "baseline",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.starts_with("{\"error\"") {
        return None;
    }
    Some(text)
}

/// Cross-platform "quote this token for the target shell" helper —
/// double-quotes and escapes embedded double quotes/backslashes the same way
/// on both `cmd /C` and `sh -c`, which both treat a double-quoted span as one
/// token.
fn shell_quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for ch in value.chars() {
        if ch == '"' || ch == '\\' {
            quoted.push('\\');
        }
        quoted.push(ch);
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
