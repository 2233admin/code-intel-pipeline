//! `code-intel perf-optimize denoise-eval` — the internal subcommand that
//! `perf-optimize run` hands to weco as its own `--eval-command`. weco calls
//! this once per step; this wraps the *real* eval command, runs it
//! `--n` times (default 5), and reprints the median in the same
//! `metric_name: value` shape weco expects, so weco's own single-sample
//! parsing never sees the raw noise.

use super::denoise;

pub(crate) fn run_raw(raw: &[String]) -> i32 {
    let cli = match parse(raw) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}");
            return 64;
        }
    };
    match denoise::run_denoised(&cli.command, cli.n) {
        Ok(result) => {
            println!("{}: {}", result.metric_name, result.median);
            0
        }
        Err(error) => {
            eprintln!("{error}");
            65
        }
    }
}

#[derive(Debug)]
struct Cli {
    command: String,
    n: usize,
}

fn parse(raw: &[String]) -> Result<Cli, String> {
    let mut command = None;
    let mut n = denoise::DEFAULT_DENOISE_N;
    let mut iter = raw.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--command" => {
                command = Some(super::next_value(&mut iter, "--command")?.clone());
            }
            "--n" => {
                n = super::next_value(&mut iter, "--n")?
                    .parse::<usize>()
                    .map_err(|_| "--n must be a positive integer".to_string())?;
            }
            other => {
                return Err(format!(
                    "unknown perf-optimize denoise-eval argument: {other}"
                ))
            }
        }
    }
    Ok(Cli {
        command: command.ok_or("--command is required")?,
        n,
    })
}

#[cfg(test)]
#[path = "denoise_eval_cli_tests.rs"]
mod tests;
