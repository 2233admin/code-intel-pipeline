//! `code-intel verify <path> [--json]` -- the aggregating structural-integrity
//! gate an agent, orchestrator, or human currently has to run as three
//! separate commands before trusting a change (issue #367):
//!
//!   1. `lint hardcoded-paths <path>` -- machine-specific paths in tracked
//!      `.ps1`/`.psm1`/`.md`/`.yml` files.
//!   2. `sentrux gate <path>` -- the structural regression gate against
//!      `.sentrux/baseline.json`.
//!   3. `repin --repo <path>` in check-only mode -- never `--write`; verify
//!      reports stale/unresolved digest pins, it never fixes them.
//!
//! Deliberately excludes `cargo test`: that is a separate, much heavier,
//! whole-workspace operation with a different cost profile. `verify` stays
//! fast and scoped to `<path>`, composing the three engines above (each
//! already exposes a check-only, non-mutating entry point) rather than
//! reimplementing any of their analysis.
//!
//! Exit codes: 0 = clean, 1 = at least one sub-check reported a real
//! violation, 64 = usage error (missing or invalid `<path>`), 74 = a
//! sub-check's engine itself could not run at all -- fail closed, this is
//! never folded into a false "ok".

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::hardcoded_paths;
use crate::repin;
use crate::sentrux_gate;

/// One sub-check's outcome. `EngineFailure` means the check itself could not
/// run (bad repo path, unreadable file, subprocess failure, ...) and is
/// never conflated with `Fail`, which is a real violation the check
/// completed and found -- `verify` must fail closed on the former without
/// ever reporting it as an ordinary business failure.
enum SubCheckOutcome {
    Pass { human: String, json: Value },
    Fail { human: String, json: Value },
    EngineFailure { message: String },
}

struct SubCheck {
    name: &'static str,
    outcome: SubCheckOutcome,
}

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let json_output = raw.iter().any(|argument| argument == "--json");
    let repo_arg = raw.iter().find(|argument| !argument.starts_with('-'));
    let repo = match repo_arg {
        Some(path) => PathBuf::from(path),
        None => return report_usage_error(json_output, "verify requires a <path> argument"),
    };
    if !repo.is_dir() {
        return report_usage_error(
            json_output,
            &format!("verify path is not a directory: {}", repo.display()),
        );
    }
    let repo = match repo.canonicalize() {
        Ok(repo) => repo,
        Err(error) => {
            return report_usage_error(json_output, &format!("resolve verify path: {error}"))
        }
    };

    let checks = [
        run_lint_check(&repo),
        run_gate_check(&repo),
        run_repin_check(&repo),
    ];

    let mut engine_failed = false;
    let mut business_failed = false;
    for check in &checks {
        match &check.outcome {
            SubCheckOutcome::Pass { .. } => {}
            SubCheckOutcome::Fail { .. } => business_failed = true,
            SubCheckOutcome::EngineFailure { .. } => engine_failed = true,
        }
    }
    let ok = !engine_failed && !business_failed;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&render_json(&checks, ok))
                .expect("verify report serializes")
        );
    } else {
        print!("{}", render_human(&checks, ok));
    }

    if engine_failed {
        74
    } else if business_failed {
        1
    } else {
        0
    }
}

/// Sub-check 1: `hardcoded_paths::scan`, the same engine `lint
/// hardcoded-paths` runs, reused as data instead of a printed CLI report.
fn run_lint_check(repo: &Path) -> SubCheck {
    let outcome = match hardcoded_paths::scan(repo) {
        Ok(result) => {
            let json = hardcoded_paths::scan_json(&result);
            let human = hardcoded_paths::render_report(&result);
            if result.ok {
                SubCheckOutcome::Pass { human, json }
            } else {
                SubCheckOutcome::Fail { human, json }
            }
        }
        Err(message) => SubCheckOutcome::EngineFailure { message },
    };
    SubCheck {
        name: "lint hardcoded-paths",
        outcome,
    }
}

/// Sub-check 2: `sentrux_gate::run_gate` in ratchet mode (`save=false`), the
/// same engine `sentrux gate` runs -- never `save=true`, which would write
/// `.sentrux/baseline.json`. A missing baseline is a real failure here, the
/// same as it is for the standalone `sentrux gate` command: absence of
/// governance is not silently treated as clean.
fn run_gate_check(repo: &Path) -> SubCheck {
    let outcome = match sentrux_gate::run_gate(repo, false) {
        Ok(run) => {
            let json = json!({
                "ok": run.success,
                "governed": run.governed,
                "violations": run
                    .violations
                    .iter()
                    .map(sentrux_gate::Violation::to_json)
                    .collect::<Vec<_>>(),
            });
            if run.success {
                SubCheckOutcome::Pass {
                    human: run.stdout,
                    json,
                }
            } else {
                SubCheckOutcome::Fail {
                    human: run.stdout,
                    json,
                }
            }
        }
        Err(message) => SubCheckOutcome::EngineFailure { message },
    };
    SubCheck {
        name: "sentrux gate",
        outcome,
    }
}

/// Sub-check 3: `repin::run_check`, the check-only entry point that runs the
/// exact same scan `repin --repo <path>` runs without `--write` -- it never
/// mutates the repository (no `flush`, no `declared_pins::resync`).
fn run_repin_check(repo: &Path) -> SubCheck {
    let outcome = match repin::run_check(repo) {
        Ok(result) => {
            if result.ok {
                SubCheckOutcome::Pass {
                    human: result.human,
                    json: result.json,
                }
            } else {
                SubCheckOutcome::Fail {
                    human: result.human,
                    json: result.json,
                }
            }
        }
        Err(message) => SubCheckOutcome::EngineFailure { message },
    };
    SubCheck {
        name: "repin (check-only)",
        outcome,
    }
}

fn render_human(checks: &[SubCheck; 3], ok: bool) -> String {
    let mut out = String::new();
    out.push_str(if ok {
        "code-intel verify: OK\n"
    } else {
        "code-intel verify: FAILED\n"
    });
    for check in checks {
        let (label, detail) = match &check.outcome {
            SubCheckOutcome::Pass { human, .. } => ("PASS", human.as_str()),
            SubCheckOutcome::Fail { human, .. } => ("FAIL", human.as_str()),
            SubCheckOutcome::EngineFailure { message } => ("ERROR", message.as_str()),
        };
        out.push_str(&format!("[{label}] {}\n", check.name));
        for line in detail.lines() {
            out.push_str("    ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn render_json(checks: &[SubCheck; 3], ok: bool) -> Value {
    json!({
        "schema": "code-intel-verify-report.v1",
        "ok": ok,
        "checks": checks
            .iter()
            .map(|check| {
                let (status, detail) = match &check.outcome {
                    SubCheckOutcome::Pass { json, .. } => ("pass", json.clone()),
                    SubCheckOutcome::Fail { json, .. } => ("fail", json.clone()),
                    SubCheckOutcome::EngineFailure { message } => {
                        ("engine_failure", json!({ "error": message }))
                    }
                };
                json!({
                    "name": check.name,
                    "status": status,
                    "detail": detail,
                })
            })
            .collect::<Vec<_>>(),
    })
}

fn report_usage_error(json_output: bool, message: &str) -> i32 {
    if json_output {
        println!(
            "{}",
            json!({
                "ok": false,
                "error": {
                    "category": "usage_error",
                    "message": message,
                }
            })
        );
    } else {
        eprintln!("verify: ERROR: {message}");
    }
    64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pass(name: &'static str) -> SubCheck {
        SubCheck {
            name,
            outcome: SubCheckOutcome::Pass {
                human: "all clear\n".into(),
                json: json!({"ok": true}),
            },
        }
    }

    fn fail(name: &'static str) -> SubCheck {
        SubCheck {
            name,
            outcome: SubCheckOutcome::Fail {
                human: "found a problem\n".into(),
                json: json!({"ok": false}),
            },
        }
    }

    fn engine_failure(name: &'static str) -> SubCheck {
        SubCheck {
            name,
            outcome: SubCheckOutcome::EngineFailure {
                message: "could not run".into(),
            },
        }
    }

    #[test]
    fn all_passing_checks_render_ok_human_summary() {
        let checks = [pass("a"), pass("b"), pass("c")];
        let human = render_human(&checks, true);
        assert!(human.starts_with("code-intel verify: OK\n"));
        assert!(human.contains("[PASS] a"));
        assert!(human.contains("[PASS] b"));
        assert!(human.contains("[PASS] c"));
        assert!(human.contains("    all clear"));
    }

    #[test]
    fn a_single_failure_renders_failed_human_summary_naming_the_check() {
        let checks = [pass("a"), fail("b"), pass("c")];
        let human = render_human(&checks, false);
        assert!(human.starts_with("code-intel verify: FAILED\n"));
        assert!(human.contains("[PASS] a"));
        assert!(human.contains("[FAIL] b"));
        assert!(human.contains("    found a problem"));
        assert!(human.contains("[PASS] c"));
    }

    #[test]
    fn engine_failure_renders_as_error_not_fail() {
        let checks = [pass("a"), engine_failure("b"), pass("c")];
        let human = render_human(&checks, false);
        assert!(human.contains("[ERROR] b"));
        assert!(human.contains("    could not run"));
        assert!(!human.contains("[FAIL] b"));
    }

    #[test]
    fn json_report_carries_ok_flag_and_per_check_status() {
        let checks = [pass("a"), fail("b"), engine_failure("c")];
        let value = render_json(&checks, false);
        assert_eq!(value["schema"], "code-intel-verify-report.v1");
        assert_eq!(value["ok"], false);
        let entries = value["checks"].as_array().expect("checks array");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0]["name"], "a");
        assert_eq!(entries[0]["status"], "pass");
        assert_eq!(entries[1]["name"], "b");
        assert_eq!(entries[1]["status"], "fail");
        assert_eq!(entries[2]["name"], "c");
        assert_eq!(entries[2]["status"], "engine_failure");
        assert_eq!(entries[2]["detail"]["error"], "could not run");
    }

    #[test]
    fn usage_error_reports_exit_64_and_stays_silent_in_json_mode_on_stderr() {
        // report_usage_error itself only decides which stream/shape to use;
        // the actual exit code contract is asserted end-to-end in
        // tests/verify.rs. This pins the return value directly.
        assert_eq!(report_usage_error(false, "missing"), 64);
        assert_eq!(report_usage_error(true, "missing"), 64);
    }
}
