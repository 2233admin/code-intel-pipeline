use crate::sentrux_analysis;
use crate::sentrux_capabilities;
use crate::sentrux_evolution;
use crate::sentrux_gate;
use crate::Result;
use std::path::Path;

pub struct Options<'a> {
    pub operation: Option<&'a str>,
    pub repo: Option<&'a Path>,
    pub json: bool,
    pub no_ratchet: bool,
}

pub fn run(options: &Options<'_>) -> Result<()> {
    let operation = options.operation.ok_or("sentrux requires an operation")?;

    if operation == "capabilities" {
        let repo = match options.repo {
            Some(repo) => repo.canonicalize().map_err(|error| {
                format!(
                    "sentrux capabilities repository '{}' is unavailable: {error}",
                    repo.display()
                )
            })?,
            None => std::env::current_dir()?,
        };
        return sentrux_capabilities::run_capabilities(&repo, options.json);
    }

    let repo = options.repo.ok_or("sentrux requires a repo/path")?;
    let repo = repo.canonicalize()?;
    match operation {
        "dsm" => {
            let snapshot = sentrux_analysis::analyze(&repo)?;
            println!("{}", serde_json::to_string(&snapshot)?);
            Ok(())
        }
        "hotspots" => {
            let doc = crate::sentrux_hotspots::hotspots(&repo)?;
            println!("{}", serde_json::to_string(&doc)?);
            Ok(())
        }
        "evolution" => {
            let doc = sentrux_evolution::evolution(&repo)?;
            println!("{}", serde_json::to_string(&doc)?);
            Ok(())
        }
        "what_if" => {
            let doc = sentrux_evolution::what_if(&repo)?;
            println!("{}", serde_json::to_string(&doc)?);
            Ok(())
        }
        "scan" => {
            let metrics = sentrux_gate::scan_json(&repo)?;
            println!("{}", serde_json::to_string_pretty(&metrics)?);
            Ok(())
        }
        "health" => {
            let metrics = sentrux_gate::scan_json(&repo)?;
            let god_files = metrics["god_file_count"].as_i64().unwrap_or(0);
            let complex = metrics["complex_fn_count"].as_i64().unwrap_or(0);
            let coupling = metrics["coupling_score"].as_f64().unwrap_or(0.0);
            let bottleneck = if god_files > 0 {
                "god_files"
            } else if complex > 0 {
                "complexity"
            } else if coupling > 20.0 {
                "coupling"
            } else {
                "none"
            };
            let health = serde_json::json!({
                "status": "ok",
                "tool": sentrux_gate::ENGINE_ID,
                "quality_signal": metrics["quality_signal"],
                "files": metrics["files"],
                "bottleneck": bottleneck,
            });
            println!("{}", serde_json::to_string_pretty(&health)?);
            Ok(())
        }
        "check" => finish(
            sentrux_gate::run_check_aligned(&repo, !options.no_ratchet)?,
            "check",
        ),
        "check_rules" => finish(sentrux_gate::run_check(&repo)?, "check"),
        "gate" => finish(sentrux_gate::run_gate(&repo, false)?, "gate"),
        "gate_save" | "save_baseline" => finish(sentrux_gate::run_gate(&repo, true)?, "gate"),
        other => Err(format!("sentrux operation not yet implemented in Rust: {other}").into()),
    }
}

fn finish(run: sentrux_gate::EngineRun, operation: &str) -> Result<()> {
    print!("{}", run.stdout);
    if run.success {
        Ok(())
    } else {
        Err(format!("sentrux {operation} reported a failing verdict").into())
    }
}
